import wit_world
from componentize_py_types import Err
from wit_world.imports import tools as _host_tools
import ast
import inspect
import json
import linecache
import textwrap
import traceback

# Stdlib surface bundled for guest code. componentize-py only includes modules
# reachable from this module's import graph, so every module guest code may use
# must be imported here, before the component is built.
import re
import math
import collections
import itertools
import functools
import random
import io
import os
import sys
import contextlib
import copy
import enum
import dataclasses
import pprint
import operator
import graphlib
import heapq
import bisect
import string
import typing
import csv
import tomllib
import html
import unicodedata
import difflib
import fnmatch
import urllib.parse
import xml.etree.ElementTree
import datetime
import calendar
import time
import decimal
import fractions
import statistics
import ipaddress
import base64
import hashlib
import hmac
import uuid
import zlib
import gzip
import struct
import array
import secrets
import pathlib

# Hidden variable name used to capture the value of the last top-level
# expression in exec mode so we can return it like eval does.
_RESULT_NAME = "__pybox_result"

# Filenames whose frames are internal to the host wrapper, not user code.
# These get stripped from tracebacks so users only see their own code's frames.
_NOISE_FILENAME_SUBSTRINGS = ("guest.py",)


def _register_source(filename: str, code: str) -> None:
    """Register source text with linecache so tracebacks can show source lines."""
    lines = code.splitlines(keepends=True)
    linecache.cache[filename] = (0, 0, lines, filename)


def _is_noise_frame(filename: str) -> bool:
    """True for frames that come from our own host wrapper, not user code."""
    return any(sub in (filename or "") for sub in _NOISE_FILENAME_SUBSTRINGS)


def _filter_stack(tb) -> list:
    """Walk a traceback object, dropping frames from our own host wrapper."""
    frames = []
    while tb is not None:
        frame = tb.tb_frame
        filename = frame.f_code.co_filename
        if not _is_noise_frame(filename):
            lineno = tb.tb_lineno
            name = frame.f_code.co_name
            line = linecache.getline(filename, lineno)
            frames.append(traceback.FrameSummary(filename, lineno, name, line=line))
        tb = tb.tb_next
    return frames


def _walk_chain(exc):
    """Yield each exception in the cause/context chain, oldest-first."""
    seen = set()
    cur = exc
    while cur is not None and id(cur) not in seen:
        seen.add(id(cur))
        yield cur
        if cur.__cause__ is not None:
            cur = cur.__cause__
        elif getattr(cur, "__suppress_context__", False):
            break
        else:
            cur = cur.__context__


def _format_exception_chain(e: BaseException) -> str:
    parts = []
    prev = None
    for exc in _walk_chain(e):
        if prev is not None:
            if prev.__cause__ is exc:
                parts.append("\nThe above exception was the direct cause of the following exception:\n\n")
            else:
                parts.append("\nDuring handling of the above exception, another exception occurred:\n\n")

        parts.append("Traceback (most recent call last):\n")
        stack = traceback.StackSummary.from_list(_filter_stack(exc.__traceback__))
        parts.extend(stack.format())
        parts.append("".join(traceback.format_exception_only(type(exc), exc)))
        prev = exc
    return "".join(parts)


def handle(e: BaseException) -> Err[str]:
    msg = _format_exception_chain(e)
    return Err(msg.rstrip())


def _set_argv(argv: list[str]) -> None:
    """Expose the caller's command line as ``sys.argv``.

    The host passes the complete argument vector, including ``argv[0]`` (the
    script name, ``-c``, or ``-``), so guest code sees the same shape it would
    under CPython.
    """
    sys.argv = list(argv)


# --- Host tools -----------------------------------------------------------
#
# Guest code reaches host tools through the `hq` namespace:
#
#     hq.search_notes(query="...", limit=10)
#
# The wrappers are built from the JSON schemas the host exposes, so adding or
# removing a tool on the host side needs no guest change. Each call re-reads
# the definitions because a sandbox instance may be reused for a different
# tool set (the plain `python` tool exposes none).

_PY_TYPES = {
    "string": str,
    "number": float,
    "integer": int,
    "boolean": bool,
    "array": list,
    "object": dict,
}


class ToolError(RuntimeError):
    """A host tool failed. The message is the tool's error text."""


def _annotation(prop: dict):
    kind = prop.get("type")
    if kind == "array":
        return list[_PY_TYPES.get(prop.get("items", {}).get("type"), object)]
    return _PY_TYPES.get(kind, object)


def _signature(schema: dict) -> inspect.Signature:
    required = set(schema.get("required", ()))
    parameters = []
    for name, prop in schema.get("properties", {}).items():
        # Optional parameters default to None so `bind` accepts omission; only
        # the arguments the caller actually passed are forwarded to the host.
        default = inspect.Parameter.empty if name in required else None
        parameters.append(
            inspect.Parameter(
                name,
                inspect.Parameter.KEYWORD_ONLY,
                default=default,
                annotation=_annotation(prop),
            )
        )
    return inspect.Signature(parameters)


def _decode(raw: str):
    """Decode a tool result, falling back to the raw string for non-JSON."""
    try:
        return json.loads(raw)
    except (TypeError, ValueError):
        return raw


def _make_tool(definition: dict):
    name = definition["name"]
    signature = _signature(definition.get("parameters", {}))

    def invoke(*args, **kwargs):
        # `bind` enforces required arguments and rejects unknown ones, so the
        # model gets ordinary Python TypeErrors instead of host-side surprises.
        # Re-raise without the inspect.py frames to keep tracebacks short.
        try:
            bound = signature.bind(*args, **kwargs)
        except TypeError as exc:
            raise TypeError(str(exc)) from None
        try:
            raw = _host_tools.call(name, json.dumps(bound.arguments))
        except Exception as exc:  # wit_world.types.Err
            raise ToolError(getattr(exc, "value", str(exc))) from None
        return _decode(raw)

    invoke.__name__ = name
    invoke.__doc__ = definition.get("description", "")
    invoke.__signature__ = signature
    return invoke


def _format_signature(definition: dict) -> str:
    schema = definition.get("parameters", {})
    required = set(schema.get("required", ()))
    parts = [
        name if name in required else f"{name}=..."
        for name in schema.get("properties", {})
    ]
    if not parts:
        return f"{definition['name']}()"
    # Every parameter is keyword-only; show the `*` so the model doesn't try
    # to call positionally.
    return f"{definition['name']}(*, {', '.join(parts)})"


def _first_line(text: str) -> str:
    stripped = text.strip()
    line = stripped.splitlines()[0] if stripped else ""
    return line if len(line) <= 120 else line[:117] + "..."


def _describe(definition: dict) -> str:
    schema = definition.get("parameters", {})
    required = set(schema.get("required", ()))
    lines = [
        _format_signature(definition),
        "",
        definition.get("description", ""),
    ]
    properties = schema.get("properties", {})
    if properties:
        lines.extend(["", "Arguments:"])
        for name, prop in properties.items():
            suffix = "" if name in required else " (optional)"
            lines.append(
                f"  {name}: {prop.get('type', 'any')}{suffix} - "
                f"{prop.get('description', '')}"
            )
    return "\n".join(lines)


class _Hq:
    """Host tools, plus `help()` for discovering their signatures."""

    def __init__(self, definitions: list[dict]):
        self._definitions = definitions
        self.ToolError = ToolError
        for definition in definitions:
            setattr(self, definition["name"], _make_tool(definition))

    def help(self, name: str | None = None) -> str:
        """Describe one tool, or list every tool and its signature."""
        if name is not None:
            for definition in self._definitions:
                if definition["name"] == name:
                    return _describe(definition)
            raise ToolError(f"unknown tool '{name}'")

        return "\n".join(
            f"{_format_signature(d)}\n    {_first_line(d.get('description', ''))}"
            for d in self._definitions
        )


def _make_hq() -> _Hq:
    """Build the `hq` namespace for this execution and expose it as a global."""
    hq = _Hq(json.loads(_host_tools.definitions()))
    globals()["hq"] = hq
    return hq


class WitWorld(wit_world.WitWorld):
    def eval(self, code: str, argv: list[str]) -> str:
        try:
            _set_argv(argv)
            _register_source("<input>", code)
            # Make the tool namespace visible to the evaluated expression.
            _make_hq()
            # Dedent so leading whitespace on the first line doesn't trip
            # Python's tokenizer (it raises IndentationError at module level).
            program = compile(textwrap.dedent(code), "<input>", "eval")
            return json.dumps(eval(program))
        except BaseException as e:
            raise handle(e)

    def exec(self, code: str, argv: list[str]) -> str:
        try:
            _set_argv(argv)
            _register_source("<input>", code)
            tree = ast.parse(textwrap.dedent(code), filename="<input>")

            # If the last top-level statement is a bare expression, rewrite it
            # as an assignment so we can capture its value to return.
            if tree.body:
                last = tree.body[-1]
                if isinstance(last, ast.Expr):
                    assign = ast.Assign(
                        targets=[ast.Name(id=_RESULT_NAME, ctx=ast.Store())],
                        value=last.value,
                    )
                    ast.copy_location(assign, last)
                    tree.body[-1] = assign
                    ast.fix_missing_locations(tree)

            compiled = compile(tree, "<input>", "exec")
            # Use the same dict for globals and locals so top-level definitions
            # behave like module level (nested function calls can see other
            # top-level names, including for recursion). Seed it with the tool
            # namespace so code can call `hq.<tool>(...)`.
            namespace: dict = {"hq": _make_hq()}
            exec(compiled, namespace, namespace)

            return json.dumps(namespace.get(_RESULT_NAME))
        except BaseException as e:
            raise handle(e)