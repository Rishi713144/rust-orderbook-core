use std::sync::{Arc, Mutex};

use actix_web::{App, HttpServer, middleware::Logger, web::Data};

use crate::{
    orderbook::Orderbook,
    routes::{create_order, delete_order, get_depth},
};

pub mod inputs;
pub mod orderbook;
pub mod outputs;
pub mod routes;

#[actix_web::main]
async fn main() -> Result<(), std::io::Error> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    HttpServer::new(move || {
        let orderbook = Arc::new(Mutex::new(Orderbook::new()));

        App::new()
            .app_data(Data::new(orderbook))
            .service(create_order)
            .service(delete_order)
            .service(get_depth)
            .wrap(Logger::default())
    })
    .workers(1)
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
