package org.example.orders;

import java.util.ArrayList;
import java.util.List;

/**
 * Keeps the orders of one customer.
 */
public final class OrderBook {
    private final List<Order> orders = new ArrayList<>();

    @Override
    public String toString() {
        return "OrderBook[" + orders.size() + "]"; // short form
    }

    public long total() {
        long sum = 0L;
        for (Order o : orders) {
            sum += o.amount();
        }
        return sum;
    }
}
