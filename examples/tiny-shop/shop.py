"""The shop's logic: a few products and how prices are shown."""

PRODUCTS = [
    ("Rubber duck", 450),
    ("Mechanical keyboard", 12900),
    ("Coffee beans (1 kg)", 2399),
]


def format_price(cents):
    """Format a price in cents as dollars, e.g. 2399 -> '$23.99'."""
    return f"${cents // 100}.{cents % 100:02d}"


def total(cents_list):
    return sum(cents_list)
