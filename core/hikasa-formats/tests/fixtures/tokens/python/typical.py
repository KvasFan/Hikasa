"""Summarise a CSV file."""
import csv
from pathlib import Path


def summarise(path: Path) -> dict[str, float]:
    totals = {}
    with path.open(newline="") as f:
        for row in csv.DictReader(f):
            key = row["name"]
            totals[key] = totals.get(key, 0.0) + float(row["amount"])
    return totals


if __name__ == "__main__":
    print(summarise(Path("data.csv")))  # debug output
