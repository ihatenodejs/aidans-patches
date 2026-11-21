# ruff: noqa
# fmt: off
# Synthetic fixture for inventory discovery. No production code or private data.
def build_parser():
    parser = argparse.ArgumentParser(prog="apk-lab")
    parser.add_argument("--workspace-root", default=".apk-lab")
    subparsers = parser.add_subparsers(dest="subcommand", required=True)
    p_alpha = subparsers.add_parser("alpha", help="Alpha")
    p_alpha.add_argument("artifact")
    p_alpha.add_argument("--json", nargs="?", const="-")
    g = p_alpha.add_mutually_exclusive_group()
    g.add_argument("--fast", action="store_true")
    g.add_argument("--slow", action="store_true")
    p_alpha.set_defaults(handler=handle_alpha)
    p_beta = subparsers.add_parser("beta")
    p_beta.add_argument("--profile", choices=["a", "b"], default="a")
    p_beta.add_argument("--count", type=int, default=3)
    p_beta.set_defaults(handler=handle_beta)
    return parser


def main(argv=None):
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
