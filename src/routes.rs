use std::sync::{Arc, Mutex};

use actix_web::{
    HttpResponse, Responder, delete, get, post,
    web::{Data, Json},
};

use crate::{
    inputs::{CreateOrderInput, DeleteOrder},
    orderbook::Orderbook,
    outputs::{CreateOrderResponse, DeleteOrderResponse},
};

#[post("/order")]
pub async fn create_order(
    body: Json<CreateOrderInput>,
    orderbook: Data<Arc<Mutex<Orderbook>>>,
) -> impl Responder {
    let price = body.0.price;
    let quantity = body.0.quantity;
    let user_id = body.0.user_id;
    let side = body.0.side;

    // maintain orderbook logic
    let mut orderbook = orderbook.lock().unwrap();

    match orderbook.create_order(price, quantity, user_id, side) {
        Ok(order_id) => HttpResponse::Ok().json(CreateOrderResponse {
            order_id: order_id.to_string(),
        }),
        Err(message) => HttpResponse::BadRequest().body(message),
    }
}

#[delete("/order")]
pub async fn delete_order(
    Json(body): Json<DeleteOrder>,
    _orderbook: Data<Arc<Mutex<Orderbook>>>,
) -> impl Responder {
    let _order_id = body.order_id;
    HttpResponse::Ok().json(DeleteOrderResponse {
        filled_qty: 0,
        average_price: 100,
    })
}

#[get("/depth")]
pub async fn get_depth(orderbook: Data<Arc<Mutex<Orderbook>>>) -> impl Responder {
    let orderbook = orderbook.lock().unwrap();
    let depth = orderbook.get_depth();
    HttpResponse::Ok().json(depth)
}
