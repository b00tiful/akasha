from leases.z_contract import lease_deadline, lease_valid


def test_deadline():
    assert lease_deadline(100) == 137
    assert lease_deadline(100, 2) == 102
    try:
        lease_deadline(100, 0)
    except ValueError:
        pass
    else:
        raise AssertionError("zero TTL must refuse")


def test_boundary():
    assert lease_valid(137, 136)
    assert not lease_valid(137, 137)
    assert not lease_valid(137, 138)
