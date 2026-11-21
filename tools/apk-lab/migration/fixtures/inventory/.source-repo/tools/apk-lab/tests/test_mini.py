# ruff: noqa
# fmt: off
# Synthetic fixture for inventory discovery. No production code or private data.
def test_plain():
    assert True


@pytest.mark.parametrize("value", [1, 2, 3])
def test_param(value):
    assert value
