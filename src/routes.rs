use std::sync::{Arc, Mutex};

use actix_web::{
    HttpRequest, HttpResponse, Responder, delete,
    error::{InternalError, JsonPayloadError},
    get,
    http::StatusCode,
    post,
    web::{self, Data, Json},
};

use crate::{
    inputs::{CreateOrderInput, DeleteOrder},
    orderbook::{CancelOrderError, Orderbook},
    outputs::{CreateOrderResponse, DeleteOrderResponse, ErrorResponse, TradeResponse},
};

pub fn configure_routes(config: &mut web::ServiceConfig) {
    config
        .app_data(web::JsonConfig::default().error_handler(json_error_handler))
        .service(create_order)
        .service(delete_order)
        .service(get_depth)
        .default_service(web::route().to(not_found));
}

pub fn json_error_handler(error: JsonPayloadError, _request: &HttpRequest) -> actix_web::Error {
    let response = HttpResponse::BadRequest().json(ErrorResponse {
        error: format!("invalid request body: {error}"),
    });

    InternalError::from_response(error, response).into()
}

pub async fn not_found() -> impl Responder {
    HttpResponse::NotFound().json(ErrorResponse {
        error: "resource not found".to_owned(),
    })
}

fn error_response(status: StatusCode, message: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(ErrorResponse {
        error: message.into(),
    })
}

#[post("/order")]
pub async fn create_order(
    body: Json<CreateOrderInput>,
    orderbook: Data<Arc<Mutex<Orderbook>>>,
) -> impl Responder {
    let input = body.into_inner();
    let mut orderbook = orderbook.lock().unwrap();

    match orderbook.create_order(input.price, input.quantity, input.user_id, input.side) {
        Ok(created) => {
            let remaining_quantity = created.order.remaining_quantity();
            let trades = created
                .trades
                .iter()
                .map(|trade| TradeResponse {
                    trade_id: trade.trade_id,
                    price: trade.price,
                    quantity: trade.quantity,
                    timestamp: trade.timestamp,
                })
                .collect();

            HttpResponse::Ok().json(CreateOrderResponse {
                order_id: created.order.order_id.to_string(),
                status: created.order.status,
                filled_quantity: created.order.filled_quantity,
                remaining_quantity,
                trades,
            })
        }
        Err(message) => error_response(StatusCode::BAD_REQUEST, message),
    }
}

#[delete("/order")]
pub async fn delete_order(
    body: Json<DeleteOrder>,
    orderbook: Data<Arc<Mutex<Orderbook>>>,
) -> impl Responder {
    let Ok(order_id) = body.order_id.parse::<u32>() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "order_id must be a positive integer",
        );
    };

    let mut orderbook = orderbook.lock().unwrap();

    match orderbook.cancel_order(order_id) {
        Ok(cancelled) => HttpResponse::Ok().json(DeleteOrderResponse {
            order_id: cancelled.order_id.to_string(),
            cancelled_quantity: cancelled.cancelled_quantity,
            filled_quantity: cancelled.filled_quantity,
            average_price: cancelled.average_price,
        }),
        Err(CancelOrderError::NotFound) => error_response(
            StatusCode::NOT_FOUND,
            CancelOrderError::NotFound.to_string(),
        ),
        Err(error) => error_response(StatusCode::CONFLICT, error.to_string()),
    }
}

#[get("/depth")]
pub async fn get_depth(orderbook: Data<Arc<Mutex<Orderbook>>>) -> impl Responder {
    let orderbook = orderbook.lock().unwrap();
    let depth = orderbook.get_depth();
    HttpResponse::Ok().json(depth)
}

#[cfg(test)]
mod tests {
    use super::configure_routes;
    use crate::{
        orderbook::Orderbook,
        outputs::{CreateOrderResponse, DeleteOrderResponse, ErrorResponse},
    };
    use actix_web::{App, http::StatusCode, test, web::Data};
    use std::sync::{Arc, Mutex};

    fn orderbook() -> Data<Arc<Mutex<Orderbook>>> {
        Data::new(Arc::new(Mutex::new(Orderbook::new())))
    }

    #[actix_web::test]
    async fn malformed_json_returns_error_body() {
        let app =
            test::init_service(App::new().app_data(orderbook()).configure(configure_routes)).await;

        let request = test::TestRequest::post()
            .uri("/order")
            .insert_header(("content-type", "application/json"))
            .set_payload("{ not json }")
            .to_request();

        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: ErrorResponse = test::read_body_json(response).await;
        assert!(body.error.contains("invalid request body"));
    }

    #[actix_web::test]
    async fn cancelling_unknown_order_returns_not_found() {
        let app =
            test::init_service(App::new().app_data(orderbook()).configure(configure_routes)).await;

        let request = test::TestRequest::delete()
            .uri("/order")
            .set_json(serde_json::json!({ "order_id": "999" }))
            .to_request();

        let response = test::call_service(&app, request).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: ErrorResponse = test::read_body_json(response).await;
        assert_eq!(body.error, "order not found");
    }

    #[actix_web::test]
    async fn create_order_reports_trades() {
        let app =
            test::init_service(App::new().app_data(orderbook()).configure(configure_routes)).await;

        test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/order")
                .set_json(serde_json::json!({
                    "price": 100,
                    "quantity": 5,
                    "user_id": 1,
                    "side": "Sell"
                }))
                .to_request(),
        )
        .await;

        let response = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/order")
                .set_json(serde_json::json!({
                    "price": 100,
                    "quantity": 5,
                    "user_id": 2,
                    "side": "Buy"
                }))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body: CreateOrderResponse = test::read_body_json(response).await;

        assert_eq!(body.filled_quantity, 5);
        assert_eq!(body.remaining_quantity, 0);
        assert_eq!(body.trades.len(), 1);
        assert_eq!(body.trades[0].price, 100);
        assert_eq!(body.trades[0].quantity, 5);
    }

    #[actix_web::test]
    async fn cancel_returns_cancelled_quantity() {
        let app =
            test::init_service(App::new().app_data(orderbook()).configure(configure_routes)).await;

        let create = test::call_service(
            &app,
            test::TestRequest::post()
                .uri("/order")
                .set_json(serde_json::json!({
                    "price": 100,
                    "quantity": 5,
                    "user_id": 1,
                    "side": "Sell"
                }))
                .to_request(),
        )
        .await;
        let created: CreateOrderResponse = test::read_body_json(create).await;

        let response = test::call_service(
            &app,
            test::TestRequest::delete()
                .uri("/order")
                .set_json(serde_json::json!({ "order_id": created.order_id }))
                .to_request(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body: DeleteOrderResponse = test::read_body_json(response).await;

        assert_eq!(body.cancelled_quantity, 5);
        assert_eq!(body.filled_quantity, 0);
        assert_eq!(body.average_price, None);
    }
}
