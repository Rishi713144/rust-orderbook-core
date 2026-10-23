use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    time::SystemTime,
};

use crate::{inputs::Side, outputs::Depth};

pub struct Orderbook {
    pub bids: BTreeMap<u32, VecDeque<Order>>,
    pub asks: BTreeMap<u32, VecDeque<Order>>,
    pub orders: HashMap<u32, Order>,
    pub order_id_counter: u32,
    pub trade_id_counter: u32,
}

#[derive(Clone, Debug)]
pub struct Order {
    pub order_id: u32,
    pub user_id: u32,
    pub quantity: u32,
    pub filled_quantity: u32,
    pub price: u32,
    pub side: Side,
    pub status: OrderStatus,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderStatus {
    Active,
    Filled,
    PartiallyFilled,
    Cancelled,
}

pub struct Trade {
    pub trade_id: u32,
    pub buy_order_id: u32,
    pub sell_order_id: u32,
    pub quantity: u32,
    pub price: u32,
    pub timestamp: u64,
}

pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

impl Orderbook {
    pub fn new() -> Self {
        Self {
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            orders: HashMap::new(),
            order_id_counter: 0,
            trade_id_counter: 0,
        }
    }

    pub fn create_order(&mut self, price: u32, quantity: u32, user_id: u32, side: Side) -> u32 {
        self.order_id_counter += 1;

        let order = Order {
            order_id: self.order_id_counter,
            user_id,
            quantity,
            filled_quantity: 0,
            price,
            side,
            status: OrderStatus::Active,
            timestamp: current_timestamp(),
        };

        let order_id = order.order_id;

        self.orders.insert(order.order_id, order.clone());

        let _trades = self.process_limit_order(order);

        order_id
    }

    pub fn get_depth(&self) -> Depth {
        let mut bids = Vec::new();
        let mut asks = Vec::new();

        for (price, orders) in self.bids.iter().rev() {
            let total_quantity: u32 = orders.iter().map(|order| order.remaining_quantity()).sum();
            if total_quantity > 0 {
                bids.push([*price, total_quantity]);
            }
        }

        for (price, orders) in self.asks.iter() {
            let total_quantity: u32 = orders.iter().map(|order| order.remaining_quantity()).sum();
            if total_quantity > 0 {
                asks.push([*price, total_quantity]);
            }
        }

        Depth {
            bids,
            asks,
            last_update_id: self.order_id_counter.to_string(),
        }
    }

    pub fn process_limit_order(&mut self, order: Order) -> Vec<Trade> {
        let mut order = order;

        let trades = match order.side {
            Side::Buy => self.match_buy_order(&mut order),
            Side::Sell => self.match_sell_order(&mut order),
        };

        if order.remaining_quantity() > 0 {
            self.add_order_to_book(order);
        }

        trades
    }

    pub fn add_order_to_book(&mut self, order: Order) {
        match order.side {
            Side::Buy => {
                self.bids
                    .entry(order.price)
                    .or_insert_with(VecDeque::new)
                    .push_back(order);
            }
            Side::Sell => {
                self.asks
                    .entry(order.price)
                    .or_insert_with(VecDeque::new)
                    .push_back(order);
            }
        }
    }

    pub fn match_buy_order(&mut self, buy_order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();

        let matchable_asks: Vec<u32> = self
            .asks
            .range(..=buy_order.price)
            .map(|(price, _)| *price)
            .collect();

        for ask_price in matchable_asks {
            while buy_order.remaining_quantity() > 0 {
                let Some(mut sell_order) = self
                    .asks
                    .get_mut(&ask_price)
                    .and_then(|queue| queue.pop_front())
                else {
                    break;
                };

                let trade_quantity = std::cmp::min(
                    buy_order.remaining_quantity(),
                    sell_order.remaining_quantity(),
                );

                let trade = Trade {
                    trade_id: self.next_trade_id(),
                    quantity: trade_quantity,
                    price: ask_price,
                    buy_order_id: buy_order.order_id,
                    sell_order_id: sell_order.order_id,
                    timestamp: current_timestamp(),
                };

                buy_order.filled_quantity += trade_quantity;
                sell_order.filled_quantity += trade_quantity;

                Self::update_order_status(buy_order);
                Self::update_order_status(&mut sell_order);

                self.orders.insert(buy_order.order_id, buy_order.clone());
                self.orders.insert(sell_order.order_id, sell_order.clone());

                trades.push(trade);

                if sell_order.remaining_quantity() > 0 {
                    if let Some(queue) = self.asks.get_mut(&ask_price) {
                        queue.push_front(sell_order);
                    }
                }
            }

            if self.asks.get(&ask_price).is_some_and(VecDeque::is_empty) {
                self.asks.remove(&ask_price);
            }
        }

        trades
    }

    pub fn match_sell_order(&mut self, sell_order: &mut Order) -> Vec<Trade> {
        let mut trades = Vec::new();

        let matchable_bids: Vec<u32> = self
            .bids
            .range(sell_order.price..)
            .map(|(price, _)| *price)
            .collect();

        for bid_price in matchable_bids {
            while sell_order.remaining_quantity() > 0 {
                let Some(mut buy_order) = self
                    .bids
                    .get_mut(&bid_price)
                    .and_then(|queue| queue.pop_front())
                else {
                    break;
                };

                let trade_quantity = std::cmp::min(
                    buy_order.remaining_quantity(),
                    sell_order.remaining_quantity(),
                );

                let trade = Trade {
                    trade_id: self.next_trade_id(),
                    quantity: trade_quantity,
                    price: bid_price,
                    buy_order_id: buy_order.order_id,
                    sell_order_id: sell_order.order_id,
                    timestamp: current_timestamp(),
                };

                buy_order.filled_quantity += trade_quantity;
                sell_order.filled_quantity += trade_quantity;

                Self::update_order_status(&mut buy_order);
                Self::update_order_status(sell_order);

                self.orders.insert(buy_order.order_id, buy_order.clone());
                self.orders.insert(sell_order.order_id, sell_order.clone());

                trades.push(trade);

                if buy_order.remaining_quantity() > 0 {
                    if let Some(queue) = self.bids.get_mut(&bid_price) {
                        queue.push_front(buy_order);
                    }
                }
            }

            if self.bids.get(&bid_price).is_some_and(VecDeque::is_empty) {
                self.bids.remove(&bid_price);
            }
        }

        trades
    }

    pub fn next_trade_id(&mut self) -> u32 {
        self.trade_id_counter += 1;
        self.trade_id_counter
    }

    pub fn update_order_status(order: &mut Order) {
        if order.remaining_quantity() == 0 {
            order.status = OrderStatus::Filled;
        } else if order.remaining_quantity() < order.quantity {
            order.status = OrderStatus::PartiallyFilled;
        } else {
            order.status = OrderStatus::Active;
        }
    }
}

impl Order {
    pub fn remaining_quantity(&self) -> u32 {
        self.quantity - self.filled_quantity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fully_matched_orders_do_not_rest_in_book() {
        let mut book = Orderbook::new();
        let buy_id = book.create_order(100, 10, 1, Side::Buy);
        let sell_id = book.create_order(100, 10, 2, Side::Sell);

        assert!(book.bids.is_empty());
        assert!(book.asks.is_empty());
        assert_eq!(book.orders[&buy_id].status, OrderStatus::Filled);
        assert_eq!(book.orders[&sell_id].status, OrderStatus::Filled);
        assert_eq!(book.trade_id_counter, 1);
    }

    #[test]
    fn incoming_order_fully_filled_keeps_resting_order() {
        let mut book = Orderbook::new();
        let sell_id = book.create_order(100, 10, 2, Side::Sell);
        let buy_id = book.create_order(100, 5, 1, Side::Buy);

        assert!(book.bids.is_empty());
        assert_eq!(book.asks[&100].len(), 1);
        assert_eq!(book.orders[&sell_id].remaining_quantity(), 5);
        assert_eq!(book.orders[&sell_id].status, OrderStatus::PartiallyFilled);
        assert_eq!(book.orders[&buy_id].status, OrderStatus::Filled);
    }

    #[test]
    fn matches_best_price_then_time_priority() {
        let mut book = Orderbook::new();
        let first = book.create_order(100, 5, 1, Side::Sell);
        let second = book.create_order(100, 5, 2, Side::Sell);
        let worse = book.create_order(101, 5, 3, Side::Sell);

        let buy_id = book.create_order(101, 7, 4, Side::Buy);

        assert_eq!(book.orders[&first].status, OrderStatus::Filled);
        assert_eq!(book.orders[&second].filled_quantity, 2);
        assert_eq!(book.orders[&worse].filled_quantity, 0);
        assert_eq!(book.orders[&buy_id].status, OrderStatus::Filled);
        assert_eq!(book.asks[&100][0].order_id, second);
        assert_eq!(book.asks[&101][0].order_id, worse);
    }

    #[test]
    fn depth_reports_remaining_quantity_in_price_order() {
        let mut book = Orderbook::new();
        book.create_order(100, 5, 1, Side::Buy);
        book.create_order(99, 3, 2, Side::Buy);
        book.create_order(101, 4, 3, Side::Sell);
        book.create_order(102, 2, 4, Side::Sell);

        let depth = book.get_depth();

        assert_eq!(depth.bids, vec![[100, 5], [99, 3]]);
        assert_eq!(depth.asks, vec![[101, 4], [102, 2]]);
        assert_eq!(depth.last_update_id, "4");
    }
}
