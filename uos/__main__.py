import os
import sys

from .cli import main

try:
    code = main()
    sys.stdout.flush()
except BrokenPipeError:
    # L'output è stato troncato (es. "uos doctor | head"): esci senza traceback
    os.dup2(os.open(os.devnull, os.O_WRONLY), sys.stdout.fileno())
    code = 0
sys.exit(code)
