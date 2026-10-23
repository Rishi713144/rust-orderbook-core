use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::{inputs::Side, outputs::Depth};

#[derive(Default)]
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
    pub filled_notional: u128,
    pub price: u32,
    pub side: Side,
    pub status: OrderStatus,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Active,
    Filled,
    PartiallyFilled,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct Trade {
    pub trade_id: u32,
    pub buy_order_id: u32,
    pub sell_order_id: u32,
    pub quantity: u32,
    pub price: u32,
    pub timestamp: u64,
}

/// Outcome of submitting a limit order: the resulting order state and any fills it produced.
#[derive(Clone, Debug)]
pub struct CreatedOrder {
    pub order: Order,
    pub trades: Vec<Trade>,
}

/// Details of an order that was successfully cancelled.
#[derive(Clone, Debug)]
pub struct CancelledOrder {
    pub order_id: u32,
    pub cancelled_quantity: u32,
    pub filled_quantity: u32,
    pub average_price: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelOrderError {
    NotFound,
    AlreadyFilled,
    AlreadyCancelled,
}

impl std::fmt::Display for CancelOrderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("order not found"),
            Self::AlreadyFilled => f.write_str("order is already filled"),
            Self::AlreadyCancelled => f.write_str("order is already cancelled"),
        }
    }
}

pub fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("Time went backwards")
        .as_millis() as u64
}

impl Orderbook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_order(
        &mut self,
        price: u32,
        quantity: u32,
        user_id: u32,
        side: Side,
    ) -> Result<CreatedOrder, &'static str> {
        if quantity == 0 {
            return Err("quantity must be greater than zero");
        }

        self.order_id_counter += 1;

        let order = Order {
            order_id: self.order_id_counter,
            user_id,
            quantity,
            filled_quantity: 0,
            filled_notional: 0,
            price,
            side,
            status: OrderStatus::Active,
            timestamp: current_timestamp(),
        };

        let order_id = order.order_id;

        self.orders.insert(order.order_id, order.clone());

        let trades = self.process_limit_order(order);
        let order = self
            .orders
            .get(&order_id)
            .cloned()
            .expect("order is inserted before matching");

        Ok(CreatedOrder { order, trades })
    }

    pub fn cancel_order(&mut self, order_id: u32) -> Result<CancelledOrder, CancelOrderError> {
        let order = self
            .orders
            .get(&order_id)
            .ok_or(CancelOrderError::NotFound)?;

        match order.status {
            OrderStatus::Active | OrderStatus::PartiallyFilled => {}
            OrderStatus::Filled => return Err(CancelOrderError::AlreadyFilled),
            OrderStatus::Cancelled => return Err(CancelOrderError::AlreadyCancelled),
        }

        let price = order.price;
        let side = order.side;

        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };

        let queue = book.get_mut(&price).ok_or(CancelOrderError::NotFound)?;
        let index = queue
            .iter()
            .position(|resting| resting.order_id == order_id)
            .ok_or(CancelOrderError::NotFound)?;
        queue.remove(index);

        let queue_is_empty = queue.is_empty();
        if queue_is_empty {
            book.remove(&price);
        }

        let order = self.orders.get_mut(&order_id).expect("order exists");
        order.status = OrderStatus::Cancelled;

        Ok(CancelledOrder {
            order_id,
            cancelled_quantity: order.remaining_quantity(),
            filled_quantity: order.filled_quantity,
            average_price: order.average_price(),
        })
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
                self.bids.entry(order.price).or_default().push_back(order);
            }
            Side::Sell => {
                self.asks.entry(order.price).or_default().push_back(order);
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
            let mut skipped: VecDeque<Order> = VecDeque::new();

            while buy_order.remaining_quantity() > 0 {
                let Some(resting_order) = self
                    .asks
                    .get_mut(&ask_price)
                    .and_then(|queue| queue.pop_front())
                else {
                    break;
                };

                if resting_order.user_id == buy_order.user_id {
                    skipped.push_back(resting_order);
                    continue;
                }

                let mut sell_order = resting_order;

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
                buy_order.filled_notional += u128::from(trade_quantity) * u128::from(ask_price);
                sell_order.filled_notional += u128::from(trade_quantity) * u128::from(ask_price);

                Self::update_order_status(buy_order);
                Self::update_order_status(&mut sell_order);

                self.orders.insert(buy_order.order_id, buy_order.clone());
                self.orders.insert(sell_order.order_id, sell_order.clone());

                trades.push(trade);

                if sell_order.remaining_quantity() > 0
                    && let Some(queue) = self.asks.get_mut(&ask_price)
                {
                    queue.push_front(sell_order);
                }
            }

            if let Some(queue) = self.asks.get_mut(&ask_price) {
                while let Some(order) = skipped.pop_back() {
                    queue.push_front(order);
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
            .rev()
            .map(|(price, _)| *price)
            .collect();

        for bid_price in matchable_bids {
            let mut skipped: VecDeque<Order> = VecDeque::new();

            while sell_order.remaining_quantity() > 0 {
                let Some(resting_order) = self
                    .bids
                    .get_mut(&bid_price)
                    .and_then(|queue| queue.pop_front())
                else {
                    break;
                };

                if resting_order.user_id == sell_order.user_id {
                    skipped.push_back(resting_order);
                    continue;
                }

                let mut buy_order = resting_order;

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
                buy_order.filled_notional += u128::from(trade_quantity) * u128::from(bid_price);
                sell_order.filled_notional += u128::from(trade_quantity) * u128::from(bid_price);

                Self::update_order_status(&mut buy_order);
                Self::update_order_status(sell_order);

                self.orders.insert(buy_order.order_id, buy_order.clone());
                self.orders.insert(sell_order.order_id, sell_order.clone());

                trades.push(trade);

                if buy_order.remaining_quantity() > 0
                    && let Some(queue) = self.bids.get_mut(&bid_price)
                {
                    queue.push_front(buy_order);
                }
            }

            if let Some(queue) = self.bids.get_mut(&bid_price) {
                while let Some(order) = skipped.pop_back() {
                    queue.push_front(order);
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

    pub fn average_price(&self) -> Option<u32> {
        if self.filled_quantity == 0 {
            return None;
        }

        Some((self.filled_notional / u128::from(self.filled_quantity)) as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(book: &mut Orderbook, price: u32, quantity: u32, user_id: u32, side: Side) -> u32 {
        book.create_order(price, quantity, user_id, side)
            .unwrap()
            .order
            .order_id
    }

    #[test]
    fn fully_matched_orders_do_not_rest_in_book() {
        let mut book = Orderbook::new();
        let buy_id = place(&mut book, 100, 10, 1, Side::Buy);
        let sell_id = place(&mut book, 100, 10, 2, Side::Sell);

        assert!(book.bids.is_empty());
        assert!(book.asks.is_empty());
        assert_eq!(book.orders[&buy_id].status, OrderStatus::Filled);
        assert_eq!(book.orders[&sell_id].status, OrderStatus::Filled);
        assert_eq!(book.trade_id_counter, 1);
    }

    #[test]
    fn incoming_order_fully_filled_keeps_resting_order() {
        let mut book = Orderbook::new();
        let sell_id = place(&mut book, 100, 10, 2, Side::Sell);
        let buy_id = place(&mut book, 100, 5, 1, Side::Buy);

        assert!(book.bids.is_empty());
        assert_eq!(book.asks[&100].len(), 1);
        assert_eq!(book.orders[&sell_id].remaining_quantity(), 5);
        assert_eq!(book.orders[&sell_id].status, OrderStatus::PartiallyFilled);
        assert_eq!(book.orders[&buy_id].status, OrderStatus::Filled);
    }

    #[test]
    fn matches_best_price_then_time_priority() {
        let mut book = Orderbook::new();
        let first = place(&mut book, 100, 5, 1, Side::Sell);
        let second = place(&mut book, 100, 5, 2, Side::Sell);
        let worse = place(&mut book, 101, 5, 3, Side::Sell);

        let buy_id = place(&mut book, 101, 7, 4, Side::Buy);

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
        place(&mut book, 100, 5, 1, Side::Buy);
        place(&mut book, 99, 3, 2, Side::Buy);
        place(&mut book, 101, 4, 3, Side::Sell);
        place(&mut book, 102, 2, 4, Side::Sell);

        let depth = book.get_depth();

        assert_eq!(depth.bids, vec![[100, 5], [99, 3]]);
        assert_eq!(depth.asks, vec![[101, 4], [102, 2]]);
        assert_eq!(depth.last_update_id, "4");
    }

    #[test]
    fn sell_matches_highest_bid_first() {
        let mut book = Orderbook::new();
        let low = place(&mut book, 100, 5, 1, Side::Buy);
        let high = place(&mut book, 101, 5, 2, Side::Buy);
        let sell_id = place(&mut book, 100, 5, 3, Side::Sell);

        assert_eq!(book.orders[&high].status, OrderStatus::Filled);
        assert_eq!(book.orders[&low].filled_quantity, 0);
        assert_eq!(book.orders[&sell_id].status, OrderStatus::Filled);
        assert!(book.asks.is_empty());
        assert_eq!(book.bids[&100][0].order_id, low);
    }

    #[test]
    fn rejects_zero_quantity_orders() {
        let mut book = Orderbook::new();

        assert!(book.create_order(100, 0, 1, Side::Buy).is_err());
        assert_eq!(book.order_id_counter, 0);
        assert!(book.orders.is_empty());
    }

    #[test]
    fn same_user_orders_do_not_match() {
        let mut book = Orderbook::new();
        let sell_id = place(&mut book, 100, 10, 1, Side::Sell);
        let buy_id = place(&mut book, 100, 5, 1, Side::Buy);

        assert_eq!(book.trade_id_counter, 0);
        assert_eq!(book.orders[&sell_id].filled_quantity, 0);
        assert_eq!(book.orders[&buy_id].filled_quantity, 0);
        assert_eq!(book.asks[&100][0].order_id, sell_id);
        assert_eq!(book.bids[&100][0].order_id, buy_id);
    }

    #[test]
    fn create_order_returns_resulting_fills() {
        let mut book = Orderbook::new();
        place(&mut book, 100, 10, 1, Side::Sell);

        let created = book.create_order(100, 4, 2, Side::Buy).unwrap();

        assert_eq!(created.order.status, OrderStatus::Filled);
        assert_eq!(created.order.filled_quantity, 4);
        assert_eq!(created.order.remaining_quantity(), 0);
        assert_eq!(created.order.average_price(), Some(100));
        assert_eq!(created.trades.len(), 1);
        assert_eq!(created.trades[0].quantity, 4);
        assert_eq!(created.trades[0].price, 100);
        assert_eq!(book.asks[&100][0].remaining_quantity(), 6);
    }

    #[test]
    fn average_price_reflects_multi_level_fills() {
        let mut book = Orderbook::new();
        place(&mut book, 100, 5, 1, Side::Sell);
        place(&mut book, 102, 5, 2, Side::Sell);

        let created = book.create_order(102, 10, 3, Side::Buy).unwrap();

        assert_eq!(created.order.filled_quantity, 10);
        assert_eq!(created.order.average_price(), Some(101));
        assert_eq!(created.trades.len(), 2);
    }

    #[test]
    fn cancel_removes_remaining_quantity_from_book() {
        let mut book = Orderbook::new();
        let sell_id = place(&mut book, 100, 10, 1, Side::Sell);
        let other = place(&mut book, 100, 5, 2, Side::Sell);
        place(&mut book, 101, 7, 3, Side::Sell);

        let cancelled = book.cancel_order(sell_id).unwrap();

        assert_eq!(cancelled.cancelled_quantity, 10);
        assert_eq!(cancelled.filled_quantity, 0);
        assert_eq!(cancelled.average_price, None);
        assert_eq!(book.orders[&sell_id].status, OrderStatus::Cancelled);
        assert_eq!(book.asks[&100].len(), 1);
        assert_eq!(book.asks[&100][0].order_id, other);
        assert_eq!(book.get_depth().asks, vec![[100, 5], [101, 7]]);
    }

    #[test]
    fn cancel_removes_empty_price_level() {
        let mut book = Orderbook::new();
        let sell_id = place(&mut book, 100, 10, 1, Side::Sell);

        book.cancel_order(sell_id).unwrap();

        assert!(book.asks.is_empty());
        assert!(book.get_depth().asks.is_empty());
    }

    #[test]
    fn cancel_partially_filled_order_reports_fill() {
        let mut book = Orderbook::new();
        let sell_id = place(&mut book, 100, 10, 1, Side::Sell);
        place(&mut book, 100, 4, 2, Side::Buy);

        let cancelled = book.cancel_order(sell_id).unwrap();

        assert_eq!(cancelled.cancelled_quantity, 6);
        assert_eq!(cancelled.filled_quantity, 4);
        assert_eq!(cancelled.average_price, Some(100));
        assert!(book.asks.is_empty());
    }

    #[test]
    fn cancel_rejects_unknown_and_unavailable_orders() {
        let mut book = Orderbook::new();

        assert!(matches!(
            book.cancel_order(999),
            Err(CancelOrderError::NotFound)
        ));

        let filled = place(&mut book, 100, 10, 1, Side::Sell);
        place(&mut book, 100, 10, 2, Side::Buy);
        assert!(matches!(
            book.cancel_order(filled),
            Err(CancelOrderError::AlreadyFilled)
        ));

        let cancelled = place(&mut book, 101, 5, 3, Side::Sell);
        book.cancel_order(cancelled).unwrap();
        assert!(matches!(
            book.cancel_order(cancelled),
            Err(CancelOrderError::AlreadyCancelled)
        ));
    }
}
