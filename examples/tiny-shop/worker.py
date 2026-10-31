"""A pretend background worker that processes an order every few seconds."""

import itertools
import os
import random
import time

from shop import PRODUCTS, format_price

ENV = os.environ.get("APP_ENV", "dev")
LOG_DIR = os.environ.get("LOG_DIR", "logs")
os.makedirs(LOG_DIR, exist_ok=True)
print(f"Worker started ({ENV})", flush=True)

for n in itertools.count(1):
    name, cents = random.choice(PRODUCTS)
    line = f"{time.strftime('%H:%M:%S')} order #{n}: {name} ({format_price(cents)})"
    print(line, flush=True)
    with open(os.path.join(LOG_DIR, "worker.log"), "a") as f:
        f.write(line + "\n")
    time.sleep(3)
