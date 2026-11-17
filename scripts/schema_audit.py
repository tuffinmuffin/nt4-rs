#!/usr/bin/env python3
"""Check nt4-wpimath's struct types against WPILib's own struct schemas.

Usage: python3 scripts/schema_audit.py <allwpilib checkout> [--ours FILE] [--robotpy PYTHON]

Reads every struct type WPILib defines in the places listed in CPP_GLOBS
and JAVA_GLOBS: its type name, its schema and (C++ only) the structs it
nests, in order. Then gets what nt4-wpimath's types report, by running
`cargo run -p nt4-wpimath --example audit_schemas` (or reading that
output from FILE), and compares:

- a type of ours whose schema or nested schema order differs from WPILib's
  fails;
- a type of ours WPILib doesn't have (renamed or removed?) fails;
- a WPILib type we don't have fails, unless scripts/schema_audit.txt lists
  it as `todo` (to be added) or `na` (won't be), with a reason;
- a type in nt4-wpimath's source that the example doesn't print fails, so
  the example can't fall behind the crate.

Where WPILib's C++ and Java disagree, C++ wins (nt4-wpimath matches
wpimath's `*Struct.hpp`) and the difference is printed as a note.

With `--robotpy PYTHON` (a Python with robotpy-wpimath and robotpy-hal),
it also runs scripts/robotpy_structs.py there, which asks compiled WPILib
code through RobotPy's wpistruct. A type of ours whose schema, size or
nested schemas differ from RobotPy's fails. Where RobotPy and the parsed
C++ differ, or RobotPy has a type the parse missed, that is printed as a
note: either the parser is wrong, or RobotPy is built from another
allwpilib commit than the one checked out.

Generic types (`Matrix__<Rows>_<Cols>`) have names and schemas built at
compile time; they are reported with their parameters in angle brackets.
The example prints instances of ours (`SwerveDriveKinematics__4`); each is
matched to WPILib's pattern, which gives the parameters, and checked
against WPILib's schema with them filled in (`Translation2d modules[4]`).

Written by Claude.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
LIST = REPO / "scripts" / "schema_audit.txt"
OURS_SRC = REPO / "crates" / "nt4-wpimath" / "src"

# Where WPILib defines struct types, relative to allwpilib.
CPP_GLOBS = [
    "wpimath/src/main/native/include/wpi/math/*/struct/*Struct.hpp",
    "hal/src/main/native/include/wpi/hal/DriverStationTypes.hpp",
]
JAVA_GLOBS = [
    "wpimath/src/main/java/org/wpilib/math/*/struct/*Struct.java",
    "hal/src/main/java/org/wpilib/hardware/hal/struct/*Struct.java",
]


def strip_comments(src: str) -> str:
    """Drop // and /* */ comments, keeping string literals intact."""
    out, i, n = [], 0, len(src)
    while i < n:
        if src.startswith("//", i):
            i = src.find("\n", i)
            i = n if i < 0 else i
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
        elif src[i] == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            out.append(src[i : j + 1])
            i = j + 1
        else:
            out.append(src[i])
            i += 1
    return "".join(out)


def matching(src: str, start: int, open_: str, close: str) -> int:
    """Index just past the bracket that closes the one at `start`."""
    depth = 0
    for i in range(start, len(src)):
        if src[i] == open_:
            depth += 1
        elif src[i] == close:
            depth -= 1
            if depth == 0:
                return i + 1
    raise ValueError(f"unbalanced {open_}{close}")


def function_body(block: str, name: str) -> str | None:
    m = re.search(rf"\b{name}\s*\([^)]*\)[^{{;]*\{{", block)
    if not m:
        return None
    return block[m.end() - 1 : matching(block, m.end() - 1, "{", "}")]


LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')


def cpp_string(expr: str, block: str) -> str:
    """A C++ string expression: adjacent literals, or a ct_string constant
    built with Concat (its non-literal parts become `<...>`)."""
    expr = expr.strip()
    if re.fullmatch(r'(\s*"(?:[^"\\]|\\.)*"\s*)+', expr):
        return "".join(LITERAL.findall(expr))
    m = re.search(rf"\bct_string\s+{re.escape(expr)}\s*=\s*(.*?);", block, re.S)
    if m:
        value = m.group(1).strip()
        c = re.fullmatch(r"wpi::util::Concat\((.*)\)", value, re.S)
        if c:
            parts = []
            for arg in split_args(c.group(1)):
                lit = re.fullmatch(r'"((?:[^"\\]|\\.)*)"_ct_string', arg)
                num = re.fullmatch(r"wpi::util::NumToCtString<(.*)>\(\)", arg, re.S)
                if lit:
                    parts.append(lit.group(1))
                elif num:
                    parts.append(f"<{' '.join(num.group(1).split())}>")
                else:
                    parts.append(f"<{' '.join(arg.split())}>")
            return "".join(parts)
    return f"<{' '.join(expr.split())}>"


def split_args(s: str) -> list[str]:
    """Split on top-level commas (not ones in brackets or strings)."""
    out, depth, cur, in_str = [], 0, [], False
    for ch in s:
        if ch == '"':
            in_str = not in_str
        elif in_str:
            pass
        elif ch in "(<[{":
            depth += 1
        elif ch in ")>]}":
            depth -= 1
        if ch == "," and depth == 0 and not in_str:
            out.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    if "".join(cur).strip():
        out.append("".join(cur).strip())
    return out


def returns(body: str) -> list[str]:
    """The expression of each `return` in `body` (a `;` inside a string
    literal doesn't end one)."""
    out = []
    for m in re.finditer(r"\breturn\b", body):
        i, in_str = m.end(), False
        while i < len(body):
            ch = body[i]
            if in_str and ch == "\\":
                i += 2
                continue
            if ch == '"':
                in_str = not in_str
            elif ch == ";" and not in_str:
                break
            i += 1
        out.append(body[m.end() : i])
    return out


def cpp_returns(body: str, block: str) -> list[str]:
    return [cpp_string(e, block) for e in returns(body)]


def short_type(cpp_type: str) -> str:
    """`wpi::math::Rotation2d` -> `Rotation2d`; templates keep their
    arguments in angle brackets."""
    t = " ".join(cpp_type.split())
    head, sep, rest = t.partition("<")
    return head.split("::")[-1] + (sep + rest if sep else "")


def cpp_structs(allwpilib: pathlib.Path) -> dict[str, dict]:
    found = {}
    for pattern in CPP_GLOBS:
        for path in sorted(allwpilib.glob(pattern)):
            src = strip_comments(path.read_text())
            for m in re.finditer(r"struct\s+(?:WPILIB_DLLEXPORT\s+)?wpi::util::Struct\s*<", src):
                lt = m.end() - 1
                end_arg = matching(src, lt, "<", ">")
                brace = src.find("{", end_arg)
                if src[end_arg:brace].strip() not in ("", "final"):
                    continue  # a forward declaration or something else
                block = src[brace : matching(src, brace, "{", "}")]
                names_body = function_body(block, "GetTypeName")
                schema_body = function_body(block, "GetSchema")
                if names_body is None or schema_body is None:
                    continue
                schemas = cpp_returns(schema_body, block)
                nested_body = function_body(block, "ForEachNested") or ""
                nested = []
                for n in re.finditer(r"ForEachStructSchema\s*<", nested_body):
                    close = matching(nested_body, n.end() - 1, "<", ">")
                    nested.append(short_type(nested_body[n.end() : close - 1]))
                rel = str(path.relative_to(allwpilib))
                for name in cpp_returns(names_body, block):
                    found[name] = {
                        "schema": schemas[0] if len(schemas) == 1 else " | ".join(schemas),
                        "nested": nested,
                        "file": rel,
                    }
    return found


def java_string(expr: str) -> str:
    """A Java string expression: literals joined with `+`; anything else
    becomes `<...>`."""
    parts = []
    for term in split_plus(expr):
        lits = re.fullmatch(r'"((?:[^"\\]|\\.)*)"', term)
        parts.append(lits.group(1) if lits else f"<{' '.join(term.split())}>")
    return "".join(parts)


def split_plus(expr: str) -> list[str]:
    out, depth, cur, in_str = [], 0, [], False
    for ch in expr:
        if ch == '"':
            in_str = not in_str
        elif not in_str and ch in "([":
            depth += 1
        elif not in_str and ch in ")]":
            depth -= 1
        if ch == "+" and depth == 0 and not in_str:
            out.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    out.append("".join(cur).strip())
    return out


def java_structs(allwpilib: pathlib.Path) -> dict[str, dict]:
    found = {}
    for pattern in JAVA_GLOBS:
        for path in sorted(allwpilib.glob(pattern)):
            src = strip_comments(path.read_text())
            names = function_body(src, "getTypeName")
            schema = function_body(src, "getSchema")
            if names is None or schema is None:
                continue
            for expr in returns(names):
                found[java_string(expr)] = {
                    "schema": java_string(returns(schema)[0]),
                    "file": str(path.relative_to(allwpilib)),
                }
    return found


def upstream_structs(allwpilib: pathlib.Path) -> tuple[dict[str, dict], list[str]]:
    """WPILib's types by name (C++, then Java-only ones), and notes on where
    C++ and Java disagree."""
    cpp = cpp_structs(allwpilib)
    java = java_structs(allwpilib)
    if not cpp or not java:
        raise SystemExit(f"found no struct types under {allwpilib}; is it an allwpilib checkout?")
    notes = []
    # Generic names match whatever their parameters are called.
    cpp_generic = {placeholders(n): n for n in cpp if "<" in n}
    for name, j in sorted(java.items()):
        c = cpp.get(name) or cpp.get(cpp_generic.get(placeholders(name), ""))
        if c is None:
            cpp[name] = {"schema": j["schema"], "nested": None, "file": j["file"]}
        elif c["schema"] != j["schema"] and "<" not in c["schema"] + j["schema"]:
            notes.append(
                f"{name}: C++ and Java schemas differ\n"
                f"    C++  ({c['file']}): {c['schema']!r}\n"
                f"    Java ({j['file']}): {j['schema']!r}"
            )
    java_names = {placeholders(n) for n in java}
    for name, c in sorted(cpp.items()):
        if placeholders(name) not in java_names and c["nested"] is not None:
            notes.append(f"{name}: C++ only ({c['file']})")
    return cpp, notes


PARAM = re.compile(r"<([^<>]*)>")


def instance_params(pattern: str, name: str) -> dict[str, int] | None:
    """The parameters that make generic `pattern` (`Matrix__<Rows>_<Cols>`)
    into `name` (`Matrix__2_3`), or None if it doesn't match."""
    regex, seen, pos = [], {}, 0
    for m in PARAM.finditer(pattern):
        regex.append(re.escape(pattern[pos : m.start()]))
        param = m.group(1).strip()
        if not re.fullmatch(r"\w+", param):
            regex.append(r"\d+")
        elif param in seen:
            regex.append(f"(?P={seen[param]})")
        else:
            seen[param] = f"p{len(seen)}"
            regex.append(f"(?P<{seen[param]}>\\d+)")
        pos = m.end()
    regex.append(re.escape(pattern[pos:]))
    m = re.fullmatch("".join(regex), name)
    if m is None:
        return None
    return {param: int(m.group(group)) for param, group in seen.items()}


def fill(pattern: str, params: dict[str, int]) -> str | None:
    """`pattern` with each `<expr>` (`<Rows * Cols>`) worked out from
    `params`; None if an expression uses anything else."""
    def value(m: re.Match) -> str:
        expr = m.group(1)
        names = set(re.findall(r"[A-Za-z_]\w*", expr))
        if not re.fullmatch(r"[\w\s*+\-()]+", expr) or not names <= set(params):
            raise KeyError(expr)
        return str(eval(expr, {"__builtins__": {}}, dict(params)))  # integers and + - * only

    try:
        return PARAM.sub(value, pattern)
    except KeyError:
        return None


def resolve(name: str, upstream: dict[str, dict]) -> tuple[str | None, dict[str, int]]:
    """The upstream entry for one of our type names: itself, or the generic
    pattern it is an instance of, with the parameters."""
    if name in upstream:
        return name, {}
    for key in upstream:
        if "<" in key:
            params = instance_params(key, name)
            if params is not None:
                return key, params
    return None, {}


def placeholders(name: str) -> str:
    """`Matrix__<Rows>_<Cols>` -> `Matrix__<>_<>`."""
    return re.sub(r"<[^<>]*>", "<>", name)


def expanded_nested(name: str, upstream: dict[str, dict]) -> list[str] | None:
    """Every struct `name` registers before its own schema, in order, each
    once (as nt4 lists them); None if C++ doesn't say."""
    out: list[str] = []

    def visit(n: str) -> bool:
        info = upstream.get(n)
        if info is None or info["nested"] is None:
            return False
        for child in info["nested"]:
            if not visit(child):
                return False
            if child not in out:
                out.append(child)
        return True

    return out if visit(name) else None


def our_types(dump: str | None) -> list[dict]:
    if dump:
        text = pathlib.Path(dump).read_text()
    else:
        text = subprocess.run(
            ["cargo", "run", "-q", "-p", "nt4-wpimath", "--example", "audit_schemas"],
            cwd=REPO,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    return [json.loads(line) for line in text.splitlines() if line.strip()]


def robotpy_types(python: str) -> dict[str, dict]:
    text = subprocess.run(
        [python, "-I", str(REPO / "scripts" / "robotpy_structs.py")],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    found: dict[str, dict] = {}
    for line in text.splitlines():
        if line.strip():
            row = json.loads(line)
            found.setdefault(row["name"], row)  # aliases (DrivetrainSplineSample) come once
    return found


def compare_robotpy(robotpy, upstream, by_name, problems, notes) -> None:
    for name, r in sorted(robotpy.items()):
        up = upstream.get(name)
        if up is None:
            notes.append(f"{name}: in RobotPy ({r['py']}) but not found in allwpilib")
        else:
            if r["schema"] != up["schema"]:
                notes.append(
                    f"{name}: RobotPy and the parsed C++ differ\n"
                    f"    RobotPy: {r['schema']!r}\n    C++:     {up['schema']!r}"
                )
            nested = expanded_nested(name, upstream)
            if nested is not None and nested != r["nested"]:
                notes.append(
                    f"{name}: nested schemas differ between RobotPy and the parsed C++\n"
                    f"    RobotPy: {r['nested']}\n    C++:     {nested}"
                )
        ours = by_name.get(name)
        if ours is None:
            continue
        for key in ("schema", "size", "nested"):
            if ours[key] != r[key]:
                problems.append(
                    f"DIFFERS FROM ROBOTPY ({key}): {name}\n"
                    f"    ours:    {ours[key]!r}\n    RobotPy: {r[key]!r} ({r['py']})"
                )
    for name in sorted(set(by_name) - set(robotpy)):
        notes.append(f"{name}: ours, but RobotPy has no struct type by that name")


def our_source_types() -> set[str]:
    """Rust names of the NtStruct types in nt4-wpimath's source."""
    names = set()
    for path in OURS_SRC.rglob("*.rs"):
        src = path.read_text()
        names |= set(
            re.findall(r"#\[derive\([^)]*\bNtStruct\b[^)]*\)\][^;{]*?\bpub\s+(?:struct|enum)\s+(\w+)", src, re.S)
        )
        names |= set(re.findall(r"\bimpl(?:<[^>]*>)?\s+NtStruct\s+for\s+(\w+)", src))
    return names


def listed() -> dict[str, tuple[str, str]]:
    """scripts/schema_audit.txt: `<name> todo|na  # reason`."""
    out = {}
    for n, raw in enumerate(LIST.read_text().splitlines(), 1):
        line, _, reason = raw.partition("#")
        if not line.strip():
            continue
        parts = line.split()
        if len(parts) != 2 or parts[1] not in ("todo", "na"):
            raise SystemExit(f"{LIST.name}:{n}: expected `<name> todo|na  # reason`: {raw}")
        out[parts[0]] = (parts[1], reason.strip())
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("allwpilib", type=pathlib.Path)
    ap.add_argument("--ours", help="output of the audit_schemas example, instead of running it")
    ap.add_argument("--robotpy", metavar="PYTHON", help="also compare with RobotPy, run by this Python")
    args = ap.parse_args()

    upstream, notes = upstream_structs(args.allwpilib)
    ours = our_types(args.ours)
    marks = listed()
    problems = []

    by_name = {t["name"]: t for t in ours}
    printed = {re.sub(r"<.*>", "", t["rust"]).strip() for t in ours}
    for rust in sorted(our_source_types() - printed):
        problems.append(f"NOT IN THE EXAMPLE: {rust} (add it to crates/nt4-wpimath/examples/audit_schemas.rs)")

    matched = 0
    implemented = set()
    for name, t in sorted(by_name.items()):
        key, params = resolve(name, upstream)
        if key is None:
            problems.append(f"NOT UPSTREAM (renamed or removed?): {name}")
            continue
        up = upstream[key]
        if key in marks and key not in implemented:
            problems.append(f"LISTED BUT IMPLEMENTED: {key} is in {LIST.name} as {marks[key][0]}; remove it")
        implemented.add(key)
        expected = fill(up["schema"], params)
        if expected is None:
            problems.append(f"CAN'T WORK OUT SCHEMA: {name} from {up['schema']!r} ({up['file']})")
            continue
        if t["schema"] != expected:
            problems.append(
                f"SCHEMA DIFFERS: {name}\n    ours:   {t['schema']!r}\n    WPILib: {expected!r} ({up['file']})"
            )
            continue
        nested = expanded_nested(key, upstream)
        if nested is not None and t["nested"] != nested:
            problems.append(f"NESTED ORDER DIFFERS: {name}\n    ours:   {t['nested']}\n    WPILib: {nested}")
            continue
        matched += 1

    todo = []
    for name in sorted(upstream):
        if name in implemented:
            continue
        mark = marks.get(name)
        if mark is None:
            problems.append(f"NOT IMPLEMENTED: {name} ({upstream[name]['file']}); add it, or list it in {LIST.name}")
        elif mark[0] == "todo":
            todo.append(name)
    for name in sorted(set(marks) - set(upstream)):
        problems.append(f"LISTED BUT NOT UPSTREAM: {name} in {LIST.name}")

    robotpy_count = None
    if args.robotpy:
        robotpy = robotpy_types(args.robotpy)
        robotpy_count = len(robotpy)
        compare_robotpy(robotpy, upstream, by_name, problems, notes)

    na = sum(1 for name, (kind, _) in marks.items() if kind == "na" and name in upstream)
    print(
        f"{len(upstream)} WPILib struct types, {len(implemented)} of them ours "
        f"({matched} types and instances checked), "
        f"{len(todo)} to do, {na} not applicable"
        + (f"; RobotPy has {robotpy_count}" if robotpy_count is not None else "")
    )
    for note in notes:
        print(f"note: {note}")
    for p in problems:
        print(p)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
