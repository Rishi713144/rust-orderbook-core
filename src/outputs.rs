use serde::{Deserialize, Serialize};

use crate::orderbook::OrderStatus;

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateOrderResponse {
    pub order_id: String,
    pub status: OrderStatus,
    pub filled_quantity: u32,
    pub remaining_quantity: u32,
    pub trades: Vec<TradeResponse>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TradeResponse {
    pub trade_id: u32,
    pub price: u32,
    pub quantity: u32,
    pub timestamp: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeleteOrderResponse {
    pub order_id: String,
    pub cancelled_quantity: u32,
    pub filled_quantity: u32,
    pub average_price: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Depth {
    pub bids: Vec<[u32; 2]>,
    pub asks: Vec<[u32; 2]>,
    #[serde(rename = "lastUpdateId")]
    pub last_update_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorResponse {
    pub error: String,
}
