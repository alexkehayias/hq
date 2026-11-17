import wit_world
from componentize_py_types import Err
import ast
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


class WitWorld(wit_world.WitWorld):
    def eval(self, code: str) -> str:
        try:
            _register_source("<input>", code)
            # Dedent so leading whitespace on the first line doesn't trip
            # Python's tokenizer (it raises IndentationError at module level).
            program = compile(textwrap.dedent(code), "<input>", "eval")
            return json.dumps(eval(program))
        except BaseException as e:
            raise handle(e)

    def exec(self, code: str) -> str:
        try:
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
            # top-level names, including for recursion).
            namespace: dict = {}
            exec(compiled, namespace, namespace)

            return json.dumps(namespace.get(_RESULT_NAME))
        except BaseException as e:
            raise handle(e)