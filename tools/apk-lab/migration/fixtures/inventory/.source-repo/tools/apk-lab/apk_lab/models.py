# ruff: noqa
# fmt: off
# Synthetic fixture for inventory discovery. No production code or private data.
class ExitCode(IntEnum):
    SUCCESS = 0
    USAGE_OR_TOOL_ERROR = 2


class Mini:
    def to_dict(self):
        return {"alpha": 1, "beta": {"gamma": 2}, "none": None}
