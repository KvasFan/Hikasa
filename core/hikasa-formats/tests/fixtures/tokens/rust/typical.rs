//! Order totals.

use std::collections::BTreeMap;

/// Sums the amounts per customer.
#[derive(Debug, Default)]
pub struct Totals {
    by_customer: BTreeMap<String, u64>,
}

impl Totals {
    pub fn add(&mut self, customer: &str, amount: u64) {
        *self.by_customer.entry(customer.to_owned()).or_insert(0) += amount;
    }

    pub fn total(&self) -> u64 {
        self.by_customer.values().sum() // all customers
    }
}
