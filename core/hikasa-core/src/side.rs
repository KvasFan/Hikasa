//! Sides of a comparison (00-overview §2) and per-side value pairs.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One side of a comparison (00-overview §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// Left input.
    Left,
    /// Right input.
    Right,
    /// Ancestor of a 3-way merge (reserved in M1).
    Center,
    /// Merge target (reserved in M1).
    Output,
}

impl Side {
    /// The two input sides of a 2-way comparison, in canonical order.
    pub const INPUTS: [Side; 2] = [Side::Left, Side::Right];

    /// Left↔Right; `Center` and `Output` map to themselves.
    pub const fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Center => Side::Center,
            Side::Output => Side::Output,
        }
    }

    /// Stable lowercase name, identical to the serde form (`"left"`, `"right"`, `"center"`, `"output"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
            Side::Center => "center",
            Side::Output => "output",
        }
    }
}

impl fmt::Display for Side {
    /// Writes [`Side::as_str`]; used by error messages such as "left input …".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One value per input side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Pair<T> {
    /// Left value.
    pub left: T,
    /// Right value.
    pub right: T,
}

impl<T> Pair<T> {
    /// Builds a pair.
    pub const fn new(left: T, right: T) -> Self {
        Pair { left, right }
    }

    /// The value of `side`; `None` for `Center` and `Output`.
    pub fn get(&self, side: Side) -> Option<&T> {
        match side {
            Side::Left => Some(&self.left),
            Side::Right => Some(&self.right),
            Side::Center | Side::Output => None,
        }
    }

    /// Maps both values (left first: the order is observable when `f` has side effects).
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Pair<U> {
        let left = f(self.left);
        let right = f(self.right);
        Pair { left, right }
    }

    /// Borrows both values.
    pub fn as_ref(&self) -> Pair<&T> {
        Pair {
            left: &self.left,
            right: &self.right,
        }
    }

    /// Exchanges left and right.
    pub fn swapped(self) -> Pair<T> {
        Pair {
            left: self.right,
            right: self.left,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opposite_and_names() {
        assert_eq!(Side::Left.opposite(), Side::Right);
        assert_eq!(Side::Right.opposite(), Side::Left);
        assert_eq!(Side::Center.opposite(), Side::Center);
        assert_eq!(Side::Output.opposite(), Side::Output);
        assert_eq!(Side::INPUTS, [Side::Left, Side::Right]);
        for side in [Side::Left, Side::Right, Side::Center, Side::Output] {
            assert_eq!(side.to_string(), side.as_str());
        }
    }

    #[test]
    fn pair_helpers() {
        let p = Pair::new(1, 2);
        assert_eq!(p.get(Side::Left), Some(&1));
        assert_eq!(p.get(Side::Right), Some(&2));
        assert_eq!(p.get(Side::Center), None);
        assert_eq!(p.get(Side::Output), None);
        assert_eq!(p.swapped(), Pair::new(2, 1));
        assert_eq!(p.as_ref(), Pair::new(&1, &2));
        let mut order = Vec::new();
        let mapped = p.map(|v| {
            order.push(v);
            v * 10
        });
        assert_eq!(mapped, Pair::new(10, 20));
        assert_eq!(order, vec![1, 2], "left is mapped first");
    }
}
