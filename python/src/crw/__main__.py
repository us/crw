"""CLI entry point — exec crw-mcp binary."""

import os
import sys

from crw._binary import ensure_binary


def main() -> None:
    binary = ensure_binary()
    os.execvp(str(binary), [str(binary)] + sys.argv[1:])


if __name__ == "__main__":
    main()
