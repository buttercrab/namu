"""MessagePack, as tasks exchange values: the `msgpack` package's codec if it is installed, and
otherwise this one, which covers what crosses a task's boundary (nil, booleans, integers, floats,
strings, binary, arrays and maps; no extension types)."""

from __future__ import annotations

import struct

try:  # pragma: no cover - depends on the environment
    import msgpack as _fast

    def dumps(value) -> bytes:
        return _fast.packb(value, use_bin_type=True)

    def loads(data: bytes):
        return _fast.unpackb(data, raw=False, strict_map_key=False)

except ImportError:

    def dumps(value) -> bytes:
        out = bytearray()
        _pack(value, out)
        return bytes(out)

    def loads(data: bytes):
        value, end = _unpack(memoryview(data), 0)
        if end != len(data):
            raise ValueError(f"{len(data) - end} bytes after a MessagePack value")
        return value


def _pack(value, out: bytearray) -> None:
    if value is None:
        out.append(0xC0)
    elif value is True:
        out.append(0xC3)
    elif value is False:
        out.append(0xC2)
    elif isinstance(value, int):
        _pack_int(value, out)
    elif isinstance(value, float):
        out.append(0xCB)
        out += struct.pack(">d", value)
    elif isinstance(value, str):
        data = value.encode("utf-8")
        _pack_length(len(data), out, fix=(0xA0, 31), sizes=(0xD9, 0xDA, 0xDB))
        out += data
    elif isinstance(value, (bytes, bytearray, memoryview)):
        data = bytes(value)
        _pack_length(len(data), out, fix=None, sizes=(0xC4, 0xC5, 0xC6))
        out += data
    elif isinstance(value, (list, tuple)):
        _pack_length(len(value), out, fix=(0x90, 15), sizes=(None, 0xDC, 0xDD))
        for item in value:
            _pack(item, out)
    elif isinstance(value, dict):
        _pack_length(len(value), out, fix=(0x80, 15), sizes=(None, 0xDE, 0xDF))
        for key, item in value.items():
            _pack(key, out)
            _pack(item, out)
    else:
        raise TypeError(f"{type(value).__name__} is not a MessagePack value")


def _pack_int(value: int, out: bytearray) -> None:
    if 0 <= value <= 0x7F:
        out.append(value)
    elif -32 <= value < 0:
        out.append(value & 0xFF)
    elif value >= 0:
        for marker, form, limit in ((0xCC, ">B", 1 << 8), (0xCD, ">H", 1 << 16), (0xCE, ">I", 1 << 32), (0xCF, ">Q", 1 << 64)):
            if value < limit:
                out.append(marker)
                out += struct.pack(form, value)
                return
        raise OverflowError(f"{value} does not fit 64 bits")
    else:
        for marker, form, limit in ((0xD0, ">b", 1 << 7), (0xD1, ">h", 1 << 15), (0xD2, ">i", 1 << 31), (0xD3, ">q", 1 << 63)):
            if value >= -limit:
                out.append(marker)
                out += struct.pack(form, value)
                return
        raise OverflowError(f"{value} does not fit 64 bits")


def _pack_length(length: int, out: bytearray, fix, sizes) -> None:
    if fix is not None and length <= fix[1]:
        out.append(fix[0] | length)
        return
    for marker, form, limit in zip(sizes, (">B", ">H", ">I"), (1 << 8, 1 << 16, 1 << 32)):
        if marker is not None and length < limit:
            out.append(marker)
            out += struct.pack(form, length)
            return
    raise OverflowError(f"a length of {length} does not fit 32 bits")


_SCALARS = {
    0xCC: ">B", 0xCD: ">H", 0xCE: ">I", 0xCF: ">Q",
    0xD0: ">b", 0xD1: ">h", 0xD2: ">i", 0xD3: ">q",
    0xCA: ">f", 0xCB: ">d",
}
_LENGTHS = {
    0xD9: (">B", "str"), 0xDA: (">H", "str"), 0xDB: (">I", "str"),
    0xC4: (">B", "bin"), 0xC5: (">H", "bin"), 0xC6: (">I", "bin"),
    0xDC: (">H", "array"), 0xDD: (">I", "array"),
    0xDE: (">H", "map"), 0xDF: (">I", "map"),
}


def _unpack(data: memoryview, at: int):
    marker = data[at]
    at += 1
    if marker <= 0x7F:
        return marker, at
    if marker >= 0xE0:
        return marker - 0x100, at
    if 0xA0 <= marker <= 0xBF:
        return _sized("str", marker & 0x1F, data, at)
    if 0x90 <= marker <= 0x9F:
        return _sized("array", marker & 0x0F, data, at)
    if 0x80 <= marker <= 0x8F:
        return _sized("map", marker & 0x0F, data, at)
    if marker == 0xC0:
        return None, at
    if marker in (0xC2, 0xC3):
        return marker == 0xC3, at
    if marker in _SCALARS:
        form = _SCALARS[marker]
        size = struct.calcsize(form)
        return struct.unpack_from(form, data, at)[0], at + size
    if marker in _LENGTHS:
        form, kind = _LENGTHS[marker]
        size = struct.calcsize(form)
        return _sized(kind, struct.unpack_from(form, data, at)[0], data, at + size)
    raise ValueError(f"unsupported MessagePack marker 0x{marker:02x}")


def _sized(kind: str, length: int, data: memoryview, at: int):
    if kind == "str":
        return bytes(data[at : at + length]).decode("utf-8"), at + length
    if kind == "bin":
        return bytes(data[at : at + length]), at + length
    if kind == "array":
        items = []
        for _ in range(length):
            item, at = _unpack(data, at)
            items.append(item)
        return items, at
    entries = {}
    for _ in range(length):
        key, at = _unpack(data, at)
        value, at = _unpack(data, at)
        entries[key] = value
    return entries, at
