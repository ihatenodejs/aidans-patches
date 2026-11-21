# ruff: noqa
# fmt: off
# Synthetic fixture for inventory discovery. No production code or private data.
def main(argv=None):
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="subcommand", required=True)
    p_matrix = subparsers.add_parser("matrix")
    p_matrix.add_argument("--output-file")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
