"""Print every struct type RobotPy's wpimath and hal expose, one JSON object
per line: Python class, struct name, schema, size, and the schemas
registered before its own (from wpistruct.for_each_nested, own excluded).

Run by scripts/schema_audit.py with `--robotpy <python>`; that Python needs
robotpy-wpimath and robotpy-hal. RobotPy's struct support wraps the C++
`wpi::util::Struct<T>` specializations, so this is what compiled WPILib
code reports, not a parse of the headers.

Written by Claude.
"""

import importlib
import inspect
import json
import pkgutil

import wpiutil.wpistruct as ws


def modules():
    for root in ("wpimath", "hal"):
        pkg = importlib.import_module(root)
        yield pkg
        for info in pkgutil.walk_packages(pkg.__path__, root + "."):
            if "._" in info.name:
                continue  # private extension modules; re-exported publicly
            try:
                yield importlib.import_module(info.name)
            except ImportError:
                continue


def main():
    seen = set()
    rows = []
    for mod in modules():
        for obj in vars(mod).values():
            if not inspect.isclass(obj) or obj in seen:
                continue
            seen.add(obj)
            try:
                name = ws.get_type_name(obj)
            except Exception:
                continue  # not a struct type
            nested = []
            ws.for_each_nested(obj, lambda ty, _schema: nested.append(ty))
            own = f"struct:{name}"
            rows.append(
                {
                    "py": f"{obj.__module__}.{obj.__qualname__}",
                    "name": name,
                    "schema": ws.get_schema(obj),
                    "size": ws.get_size(obj),
                    "nested": [t.removeprefix("struct:") for t in nested if t != own],
                }
            )
    for row in sorted(rows, key=lambda r: (r["name"], r["py"])):
        print(json.dumps(row))


if __name__ == "__main__":
    main()
