import unittest

from shop import format_price, total


class ShopTest(unittest.TestCase):
    def test_format_price(self):
        self.assertEqual(format_price(2399), "$23.99")
        self.assertEqual(format_price(5), "$0.05")

    def test_total(self):
        self.assertEqual(total([100, 250]), 350)


if __name__ == "__main__":
    unittest.main()
