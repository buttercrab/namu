"""The signature of a task in a module, read from its annotations without running the module.

`namu build` records it in the task's manifest, so a workflow that calls the task with the wrong
types is refused when it is published, not when it runs:

    python -m namu.signature path/to/module.py task_name

prints the signature as JSON, as NAMU's IR writes one: what a call takes (its arguments as they
cross: `()` for none, the argument for one, a tuple for several) and what it gives. A stream gives
the list of its items (annotated `Iterator[T]`, or `Iterator[Item[T]]` with cursors), and a batch
task takes and gives one item.

The module is parsed, not imported, so nothing it imports needs to be installed where it is built.
Types are read as follows; anything else (no annotation, a type imported from elsewhere) is left
unknown, and not checked.

    int                         i64
    float                       f64
    str, bool, bytes, None      String, bool, bytes, ()
    list[T], set[T], Sequence[T]    Vec<T>
    tuple[A, B]                 (A, B)        tuple[T, ...]   Vec<T>
    dict[K, V], Mapping[K, V]   Map<K, V>
    T | None, Optional[T]       Option<T>
    a dataclass or TypedDict    a struct of its annotated fields, if defined in the module (a
                                TypedDict's keys that may be missing as options)

These are the annotations the worker reads values as (`namu.worker.from_value`), so what the
signature says a task takes is what it is given.
"""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path
from typing import Any

Type = Any  # A type as JSON: "string", {"list": ...}, {"named": ...}, ...

SCALARS: dict[str, Type] = {
    "int": {"int": {"bits": 64, "signed": True}},
    "float": {"float": {"bits": 64}},
    "str": "string",
    "bool": "bool",
    "bytes": "bytes",
    "bytearray": "bytes",
    "None": "unit",
    "NoneType": "unit",
}
# As the worker reads them (`namu.worker.from_value`): a task is given what it is annotated with.
LISTS = {"list", "List", "Sequence", "MutableSequence", "set", "Set", "frozenset", "FrozenSet"}
MAPS = {"dict", "Dict", "Mapping", "MutableMapping"}
ITERATORS = {"Iterator", "Iterable", "Generator", "AsyncIterator", "AsyncGenerator"}
KINDS = {"task": "single", "stream": "stream", "batch": "batch", "continuous": "continuous"}

#: Bases of an enum whose members cross by name.
ENUMS = {"Enum", "StrEnum", "IntEnum"}


def name_of(node: ast.expr) -> str | None:
    """`x` for `x` or `a.b.x`."""
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return node.attr
    return None


def unknown(what: str) -> Type:
    return {"named": what}


class Module:
    def __init__(self, path: Path):
        self.name = path.stem
        self.tree = ast.parse(path.read_text(), filename=str(path))
        self.classes = {node.name: node for node in self.tree.body if isinstance(node, ast.ClassDef)}
        self.defs: dict[str, Any] = {}

    def type(self, node: ast.expr | None) -> Type:
        """The type an annotation names."""
        if node is None:
            return unknown("")
        if isinstance(node, ast.Constant):
            if node.value is None:
                return "unit"
            if isinstance(node.value, str):  # A string annotation: "Page".
                return self.type(ast.parse(node.value, mode="eval").body)
            return unknown(repr(node.value))
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.BitOr):
            return self.union([node.left, node.right])
        if isinstance(node, ast.Subscript):
            return self.generic(name_of(node.value), node.slice)
        name = name_of(node)
        if name in SCALARS:
            return SCALARS[name]
        if name in self.classes:
            return self.named(self.classes[name])
        return unknown(ast.unparse(node))

    def generic(self, outer: str | None, arg: ast.expr) -> Type:
        args = arg.elts if isinstance(arg, ast.Tuple) else [arg]
        if outer in LISTS and len(args) == 1:
            return {"list": self.type(args[0])}
        if outer in MAPS and len(args) == 2:
            return {"map": [self.type(args[0]), self.type(args[1])]}
        if outer in ("tuple", "Tuple"):
            if len(args) == 2 and isinstance(args[1], ast.Constant) and args[1].value is Ellipsis:
                return {"list": self.type(args[0])}
            return {"tuple": [self.type(arg) for arg in args]}
        if outer == "Optional" and len(args) == 1:
            return {"option": self.type(args[0])}
        if outer == "Union":
            return self.union(args)
        if outer in ("Annotated", "Required", "NotRequired"):
            return self.type(args[0])
        return unknown(f"{outer}[{ast.unparse(arg)}]")

    def union(self, parts: list[ast.expr]) -> Type:
        """`T | None` is an option; any other union is unknown."""
        flat: list[ast.expr] = []
        for part in parts:
            if isinstance(part, ast.BinOp) and isinstance(part.op, ast.BitOr):
                flat += [part.left, part.right]
            else:
                flat.append(part)
        types = [self.type(part) for part in flat]
        rest = [ty for ty in types if ty != "unit"]
        if len(rest) == 1 and len(types) == 2:
            return {"option": rest[0]}
        return unknown(" | ".join(ast.unparse(part) for part in flat))

    def named(self, cls: ast.ClassDef) -> Type:
        """A dataclass or TypedDict as a struct of its fields, defined once. A TypedDict's keys
        that may be missing (`NotRequired[T]`, or any not `Required[T]` with `total=False`) are
        options. An `enum.Enum` is an enum of unit variants, its members by name."""
        name = f"{self.name}.{cls.name}"
        record = any(name_of(d.func if isinstance(d, ast.Call) else d) == "dataclass" for d in cls.decorator_list)
        typed_dict = any(name_of(base) == "TypedDict" for base in cls.bases)
        if any(name_of(base) in ENUMS for base in cls.bases):
            members = [
                target.id
                for stmt in cls.body
                if isinstance(stmt, ast.Assign)
                for target in stmt.targets
                if isinstance(target, ast.Name) and not target.id.startswith("_")
            ]
            self.defs[name] = {"enum": [[member, []] for member in members]}
            return {"named": name}
        if not (record or typed_dict):
            return unknown(name)
        total = not any(
            keyword.arg == "total" and isinstance(keyword.value, ast.Constant) and keyword.value.value is False
            for keyword in cls.keywords
        )
        if name not in self.defs:
            self.defs[name] = {"struct": []}
            fields = []
            for stmt in cls.body:
                if not (isinstance(stmt, ast.AnnAssign) and isinstance(stmt.target, ast.Name)):
                    continue
                annotation = stmt.annotation
                wrapper = name_of(annotation.value) if isinstance(annotation, ast.Subscript) else None
                if wrapper == "ClassVar":
                    continue
                ty = self.type(annotation)
                required = {"Required": True, "NotRequired": False}.get(wrapper or "", total)
                if typed_dict and not required and not (isinstance(ty, dict) and "option" in ty):
                    ty = {"option": ty}
                fields.append([stmt.target.id, ty])
            self.defs[name] = {"struct": fields}
        return {"named": name}

    def task(self, task: str) -> tuple[str, ast.FunctionDef | ast.ClassDef, ast.expr | None]:
        """The task named `task`, what kind it is, and the type it fails with if it says
        (`@namu.task(error=E)`), from its decorator."""
        for node in self.tree.body:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)) and node.name == task:
                for decorator in node.decorator_list:
                    called = isinstance(decorator, ast.Call)
                    kind = KINDS.get(name_of(decorator.func if called else decorator) or "")
                    if kind:
                        keywords = decorator.keywords if called else []
                        error = next((k.value for k in keywords if k.arg == "error"), None)
                        return kind, node, error
        raise SystemExit(f"{self.name} has no task {task} (a function decorated with @namu.task, ...)")

    def params(self, function: ast.FunctionDef) -> list[Type]:
        """Its positional parameters' types; `self` and keyword-only ones (a stream's `cursor`)
        are not arguments."""
        params = function.args.posonlyargs + function.args.args
        if params and params[0].arg in ("self", "cls"):
            params = params[1:]
        return [self.type(param.annotation) for param in params]

    def signature(self, task: str) -> dict[str, Any]:
        kind, node, error = self.task(task)
        signature = self.shape(kind, node, task)
        if error is not None:
            signature["error"] = self.type(error)
            if self.defs:
                signature["defs"] = self.defs
        return signature

    def shape(self, kind: str, node: ast.FunctionDef | ast.ClassDef, task: str) -> dict[str, Any]:
        """What the task takes and gives."""
        if kind == "continuous":
            assert isinstance(node, ast.ClassDef)
            step = next(
                (stmt for stmt in node.body if isinstance(stmt, ast.FunctionDef) and stmt.name == "step"),
                None,
            )
            if step is None:
                raise SystemExit(f"{task} has no step method")
            # step(admit: list[tuple[int, In]]) -> list[tuple[int, Out]]
            params = self.params(step)
            take = item_of(params[0] if params else None, 1)
            give = item_of(self.type(step.returns), 1)
            return finish([take], give, self.defs)
        assert not isinstance(node, ast.ClassDef)
        params = self.params(node)
        output = self.type(node.returns)
        if kind == "stream":
            output = self.items(node.returns)
        if kind == "batch":
            params = [item_of(params[0] if params else None)]
            output = item_of(output)
        return finish(params, output, self.defs)

    def items(self, returns: ast.expr | None) -> Type:
        """What a stream gives, as a workflow sees it: the list of its items."""
        if isinstance(returns, ast.Subscript) and name_of(returns.value) in ITERATORS:
            args = returns.slice.elts if isinstance(returns.slice, ast.Tuple) else [returns.slice]
            item = args[0]
            # An item with its cursor, `Item[T]`, crosses as its value.
            if isinstance(item, ast.Subscript) and name_of(item.value) == "Item":
                item = item.slice
            return {"list": self.type(item)}
        return unknown("")


def item_of(ty: Type | None, at: int | None = None) -> Type:
    """The item type of a list, or element `at` of the item if it is a tuple."""
    if not (isinstance(ty, dict) and "list" in ty):
        return unknown("")
    item = ty["list"]
    if at is None:
        return item
    if isinstance(item, dict) and "tuple" in item and len(item["tuple"]) > at:
        return item["tuple"][at]
    return unknown("")


def finish(params: list[Type], output: Type, defs: dict[str, Any]) -> dict[str, Any]:
    signature: dict[str, Any] = {
        "input": "unit" if not params else params[0] if len(params) == 1 else {"tuple": params},
        "output": output,
    }
    if defs:
        signature["defs"] = defs
    return signature


def main(argv: list[str]) -> None:
    if len(argv) != 3:
        raise SystemExit("usage: python -m namu.signature MODULE.py TASK")
    print(json.dumps(Module(Path(argv[1])).signature(argv[2])))


if __name__ == "__main__":
    main(sys.argv)
