#!/usr/bin/env python3
"""Generates the differential-test corpus for innerword-vim from a real Vim.

Every case is a plain buffer (one paragraph per line), a caret and a key
sequence. The script runs each case in its own Vim process (so registers,
the last search, the last change and marks cannot leak from one case to the
next), records the resulting buffer, cursor and registers, and writes one
line per case to a fixture that `tests/vimdiff.rs` replays against the
engine. The test itself does not need Vim.

Usage:
    python3 gen_vim_corpus.py OUT.tsv [--per-category N] [--seed S] [--vim PATH]
    python3 gen_vim_corpus.py OUT.tsv --keys-file FILE   # extra hand-picked cases

How the checked-in fixtures were made (Vim 9.1.1752, macOS):
    1. `gen_vim_corpus.py all.tsv --per-category 250`, plus a hand-picked
       list of about 500 cases for the usual trouble spots (`--keys-file`).
    2. `VIMDIFF_CORPUS=all.tsv VIMDIFF_SPLIT=dir cargo test -p innerword-vim
       engine_matches_vim` writes `dir/matching.tsv` and `dir/differing.tsv`.
    3. `matching.tsv` is `vim_corpus.tsv`. Every line of `differing.tsv` was
       looked at: the engine was fixed, or the case went into
       `vim_deviations.tsv` under the reason for the difference.

`--keys-file` lines are `buffer-name<TAB>line<TAB>col<TAB>keys` (1-based line,
1-based byte column, keys in the notation of `Key::parse_sequence`).

Fixture line (tab separated, see `esc` for the escapes):
    start-lines  line  col  keys  result-lines  line  col  then for each of
    the registers `" 0 1 2 - a b`: type (`v`, `V` or empty) and text; last
    a comma-separated list of tags:
    noop-undo   Vim's undo history holds a step that changed nothing.
"""

import argparse
import concurrent.futures
import json
import os
import random
import re
import subprocess
import sys
import tempfile

REGISTERS = ['"', "0", "1", "2", "-", "a", "b"]

BUFFERS = {
    "prose": [
        "The quick brown fox jumps",
        "over the lazy dog.",
        "",
        "Second para: foo(bar, baz) and [x] {y} <z>.",
        "  indented line here  ",
        "last line",
    ],
    "words": [
        "one two  three",
        "four.five,six",
        "",
        "",
        "seven eight",
        "foo--bar  ..baz!! qux",
        "a.b.c x_y_z9 end",
    ],
    "quotes": [
        'say "hello world" and \'bye now\' ok',
        "a `tick tock` b \"\" c 'x' \"two\" \"three\"",
        "if (a (b) c) { d [e] } <f g> (h)",
        "call(fn(x, y), [1, 2], {k: v})",
        "((nested (deep) here)) tail",
    ],
    "multi": [
        "héllo wörld ñandú",
        "日本語 テキスト です",
        "naïve café — dash…end",
        "e\u0301a\u0308b combining x",
        "smile 😀 face 😀😀 end",
        "",
        "über straße",
    ],
    "sentences": [
        "Hello there. How are you?  Fine!  Thanks.",
        "",
        "Another one here. And more",
        "",
        "Last sentence. Really (yes). \"Quoted.\" End.",
        "",
        "no stop",
    ],
    "sentences2": [
        "First one. Second one!  Third one?",
        "",
        "Fourth here.) Fifth \"quoted.\"  Sixth e.g. this one.",
        "",
        "",
        "Seventh... eighth.Ninth   tenth?!",
        "",
        "  Indented start. And end.  ",
        "",
        "last without stop",
    ],
    "paras": [
        "alpha beta",
        "gamma delta",
        "",
        "epsilon",
        "",
        "",
        "zeta eta theta",
        "iota",
        "",
    ],
    "short": ["x"],
    "empty": [""],
    "blank": ["   ", "", " a ", "  b"],
    "repeat": [
        "the cat and the hat and the bat",
        "The Cat sat; the cat ran, the CAT hid",
        "cat cat cat cat cat cat",
        "a,b,c,d,e,f;g;h",
        "x  y   z    w",
    ],
    "one": ["just one line of plain text here"],
}

FIND_CHARS = ["o", "e", "a", " ", ".", "(", ")", "t", "x", ","]

# Buffers in which every line of text stands between empty lines. Vim's
# sentences run across line breaks; in the engine every block is a paragraph
# and so starts a sentence. The two only agree where an empty line separates
# the paragraphs, so sentence commands are generated for these buffers only.
SENTENCE_SAFE = {"sentences", "sentences2", "short", "empty", "one"}


class Gen:
    def __init__(self, seed):
        self.r = random.Random(seed)
        self.cases = []
        self.seen = set()

    def pos(self, name):
        """A random caret (1-based line, 1-based byte column on a character)."""
        r = self.r
        lines = BUFFERS[name]
        l = r.randrange(len(lines))
        line = lines[l]
        if not line:
            return l + 1, 1
        # Character starts that are not combining marks.
        starts = [
            i
            for i, ch in enumerate(line)
            if not (0x300 <= ord(ch) <= 0x36F)
        ]
        pick = r.random()
        if pick < 0.2:
            i = starts[0]
        elif pick < 0.4:
            i = starts[-1]
        else:
            i = r.choice(starts)
        col = len(line[:i].encode("utf-8")) + 1
        return l + 1, col

    def add(self, name, keys, pos=None):
        if pos is None:
            pos = self.pos(name)
        key = (name, pos, keys)
        if key in self.seen:
            return
        self.seen.add(key)
        self.cases.append({"buf": name, "l": pos[0], "c": pos[1], "keys": keys})

    def buf(self, names=None):
        """Picks the buffer for the case being built."""
        r = self.r
        if names is None:
            names = list(BUFFERS)
            weights = [4, 4, 3, 3, 3, 3, 3, 1, 1, 1, 3, 1]
            self.cur = r.choices(names, weights)[0]
        else:
            self.cur = r.choice(names)
        return self.cur

    def safe(self):
        return self.cur in SENTENCE_SAFE

    def count(self, p=0.3):
        r = self.r
        if r.random() < p:
            return r.choice(["2", "2", "3", "3", "4", "5", "9", "1", "10"])
        return ""

    def motion(self):
        r = self.r
        simple = [
            "h", "l", "j", "k", "w", "W", "b", "B", "e", "E", "ge", "gE",
            "0", "^", "$", "g_", "gg", "G", "{", "}", "(", ")", ";", ",",
            "%", "+", "-", "_", "|", "<Space>", "<BS>", "<CR>",
        ]
        if r.random() < 0.2:
            return r.choice(["f", "F", "t", "T"]) + r.choice(FIND_CHARS)
        while True:
            m = r.choice(simple)
            if m not in ("(", ")") or self.safe():
                return m

    def cmotion(self):
        return self.count() + self.motion()

    def textobj(self):
        r = self.r
        objs = [
            "iw", "aw", "iW", "aW", "is", "as", "ip", "ap", 'i"', 'a"',
            "i'", "a'", "i`", "a`", "i(", "a(", "i)", "a)", "ib", "ab",
            "i[", "a[", "i]", "a]", "i{", "a{", "i}", "a}", "iB", "aB",
            "i<lt>", "a<lt>", "i>", "a>",
        ]
        w = [6, 6, 3, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2, 2, 3, 3, 1, 1, 1, 1,
             2, 2, 1, 1, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1]
        while True:
            o = r.choices(objs, w)[0]
            if o not in ("is", "as") or self.safe():
                return o

    def text(self):
        r = self.r
        # Typed text is run as commands when the command before it fails,
        # so sentence motions only where they compare.
        return r.choice([
            "X", "new", "ab cd", "foo ", " bar", "a.b", "Zz", "é", "日本",
            "x y z", "end.", "(p)" if self.safe() else "p.", "q", "",
        ])

    def operator(self):
        return self.r.choice(["d", "d", "d", "c", "c", "y", "y", "gu", "gU", "g~"])

    def finish_op(self, op):
        """What follows an operator's motion: text and escape after a change."""
        if op == "c":
            return self.text() + "<Esc>"
        return ""

    def change(self):
        """A complete repeatable change command."""
        r = self.r
        k = r.random()
        if k < 0.25:
            op = r.choice(["d", "c", "gU", "g~", "gu"])
            return self.count(0.15) + op + self.cmotion() + self.finish_op(op)
        if k < 0.4:
            op = r.choice(["d", "c", "gU"])
            return self.count(0.1) + op + self.count(0.1) + self.textobj() + self.finish_op(op)
        if k < 0.6:
            return self.count() + r.choice(["x", "X", "~", "J", "dd", "D", "rz", "r ", "gJ", "cc" + self.text() + "<Esc>", "S" + self.text() + "<Esc>", "s" + self.text() + "<Esc>", "C" + self.text() + "<Esc>"])
        if k < 0.85:
            return self.count(0.2) + r.choice(["i", "a", "I", "A", "o", "O"]) + self.text() + "<Esc>"
        return self.yank() + self.cmotion() + self.count(0.2) + r.choice(["p", "P", "gp", "gP"])

    def yank(self):
        r = self.r
        return r.choice([
            "yw", "yy", "yiw", "y$", "yaw", "2yy", "ye", "yl", "y2l", "yj",
            "dw", "dd", "x", "D", "diw", "2dd", "d$", "dl", "dk", "yip", "yap", "Y", "3Y",
        ])

    # ----- categories ---------------------------------------------------------------

    def cat_motions(self, n):
        for _ in range(n):
            self.buf()
            k = self.r.randrange(1, 4)
            self.add(self.cur, "".join(self.cmotion() for _ in range(k)))

    def cat_op_motion(self, n):
        for _ in range(n):
            self.buf()
            op = self.operator()
            keys = self.count(0.2) + op + self.cmotion() + self.finish_op(op)
            if self.r.random() < 0.3:
                keys = self.cmotion() + keys
            self.add(self.cur, keys)

    def cat_textobj(self, n):
        r = self.r
        for _ in range(n):
            name = self.buf(["quotes", "quotes", "prose", "words", "sentences", "sentences2", "paras", "multi", "repeat", "blank"])
            k = r.random()
            if k < 0.6:
                op = self.operator()
                keys = op + self.count(0.15) + self.textobj() + self.finish_op(op)
            elif k < 0.85:
                op = r.choice(["d", "y", "c", "U", "~"])
                keys = "v" + self.count(0.1) + self.textobj() + (self.textobj() if r.random() < 0.4 else "") + op + self.finish_op(op)
            else:
                op = r.choice(["d", "y"])
                keys = "v" + self.cmotion() + self.textobj() + op
            self.add(name, keys)

    def cat_simple(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            cmd = r.choice([
                "x", "X", "~", "J", "gJ", "D", "dd", "yy", "Y", "rz", "rQ", "r<CR>",
                "s" + self.text() + "<Esc>", "S" + self.text() + "<Esc>",
                "C" + self.text() + "<Esc>", "cc" + self.text() + "<Esc>",
                "guu", "gUU", "g~~", "gugu", "gUgU", "g~g~",
            ])
            keys = self.count(0.5) + cmd
            if r.random() < 0.3:
                keys = self.cmotion() + keys
            if r.random() < 0.3:
                keys += self.cmotion()
            self.add(self.cur, keys)

    def cat_put(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            keys = self.yank()
            if r.random() < 0.7:
                keys += self.cmotion()
            keys += self.count(0.35) + r.choice(["p", "P", "p", "P", "gp", "gP"])
            if r.random() < 0.2:
                keys += r.choice(["p", "P", "u", ".", "2."])
            self.add(self.cur, keys)

    def cat_registers(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            parts = []
            for _ in range(r.randrange(2, 5)):
                reg = r.choice(['"a', '"A', '"b', '"B', '"_', '"0', '"1', '"2', '"-', '""', "", "", ""])
                k = r.random()
                if k < 0.45:
                    if reg in ('"0', '"1', '"2', '"-') and r.random() < 0.8:
                        reg = ""
                    parts.append(reg + self.yank())
                elif k < 0.8:
                    parts.append(reg + self.count(0.15) + r.choice(["p", "P"]))
                else:
                    parts.append(self.cmotion())
            self.add(self.cur, "".join(parts))

    def cat_dot(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            keys = self.change()
            for _ in range(r.randrange(1, 3)):
                if r.random() < 0.8:
                    keys += self.cmotion()
                keys += self.count(0.25) + "."
            if r.random() < 0.15:
                keys += "u"
            self.add(self.cur, keys)

    def cat_undo(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            keys = ""
            for _ in range(r.randrange(1, 4)):
                if r.random() < 0.5:
                    keys += self.cmotion()
                keys += self.change()
            if r.random() < 0.4:
                keys += self.cmotion()
            tail = r.choice(["u", "uu", "u<C-r>", "uu<C-r>", "uu<C-r><C-r>", "2u", "u<C-r>u", "uuu", "3u2<C-r>", "u."])
            self.add(self.cur, keys + tail)

    def cat_visual(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            start = r.choice(["v", "v", "V"])
            keys = start
            for _ in range(r.randrange(0, 4)):
                keys += r.choice([self.cmotion(), self.cmotion(), "o", "v" if start == "V" else "V"])
            op = r.choice([
                "d", "x", "y", "c", "s", "~", "U", "u", "J", "gJ", "rz", "D", "X", "Y",
                "C", "S", "R", "p", "P", "gU", "gu", "g~", "<Esc>", "d", "y",
            ])
            if op in ("p", "P"):
                keys = self.yank() + self.cmotion() + keys
            keys += op
            if op in ("c", "s", "C", "S", "R"):
                keys += self.text() + "<Esc>"
            k = r.random()
            if k < 0.15:
                keys += "gv" + r.choice(["d", "y", "~", "<Esc>"])
            elif k < 0.3:
                keys += self.cmotion() + "."
            elif k < 0.4:
                keys += "u"
            elif k < 0.5:
                keys += r.choice(["`<lt>", "`>", "'<lt>", "'>"])
            self.add(self.cur, keys)

    def cat_search(self, n):
        r = self.r
        # Literal text only: the engine has no regular expressions.
        pats = ["o", "the", "fo", "The", "zzz", "e", "cat", "CAT", "a", "an", "t", "line", "é", "日", "d ", " ", ",", "one"]
        for _ in range(n):
            name = self.buf(["repeat", "repeat", "prose", "words", "multi", "sentences", "paras", "one"])
            k = r.random()
            if k < 0.45:
                keys = self.count(0.15) + r.choice(["/", "?"]) + r.choice(pats) + "<CR>"
                for _ in range(r.randrange(0, 4)):
                    keys += self.count(0.2) + r.choice(["n", "N", "n"])
            elif k < 0.7:
                keys = self.count(0.2) + r.choice(["*", "#", "g*", "g#"])
                for _ in range(r.randrange(0, 3)):
                    keys += r.choice(["n", "N", "n", "*", "#"])
            elif k < 0.9:
                op = r.choice(["d", "c", "y", "gU"])
                keys = op + r.choice(["/", "?"]) + r.choice(pats) + "<CR>" + self.finish_op(op)
                if r.random() < 0.4:
                    keys += r.choice(["n", "N", "."]) + r.choice(["", ".", "dn" , "dN"])
            else:
                keys = r.choice(["/", "?"]) + r.choice(pats) + "<CR>" + r.choice(["dn", "dN", "cnX<Esc>", "yn", "d2n", "vnd", "vNy"])
            self.add(name, keys)

    def cat_ex(self, n):
        r = self.r
        rng = ["", "", "", "%", "2", "2,3", ".,$", "1,2", "$", "1,$"]
        subs = ["s/o/0/", "s/o/0/g", "s/the/X/gi", "s/THE/X/", "s/e//g", "s/a/&&/g", "s/ /_/g", "s/cat/dog/g",
                "s/Cat/dog/g", "s/cat/dog/I", "s/zzz/y/", "s/,/!/", "s/a/b/g", "s/\\<the\\>/T/g", "s/t/tt/g", "s/x//",
                "s/an/\\0\\0/g", "s/o/\\//g", "s#o#0#g", "s/e/E"]
        for _ in range(n):
            self.buf(["repeat", "repeat", "prose", "words", "paras", "sentences"])
            k = r.random()
            if k < 0.5:
                keys = ":" + r.choice(rng) + r.choice(subs) + "<CR>"
                k2 = r.random()
                if k2 < 0.2:
                    keys += self.cmotion() + r.choice(["&", "g&", ":&&<CR>", ":s<CR>", ":%&<CR>"])
                elif k2 < 0.35:
                    keys += "u"
                elif k2 < 0.45:
                    keys += r.choice(["n", "N"])
            elif k < 0.75:
                cmd = r.choice(["d", "d", "d a", "y", "y a", "j", "j!", "d 2", "j 3", "y 2"])
                keys = ":" + r.choice(rng) + cmd + "<CR>"
                if r.random() < 0.5:
                    keys += self.cmotion() + r.choice(["p", "P", '"ap', "u", "."])
            elif k < 0.9:
                # No addresses past the last line: in batch mode Vim
                # refuses them, at the keyboard it goes to the last line.
                keys = ":" + r.choice(["1", "2", "3", "$", "0", "$-1", "-1", "-", "1+1", "$-2"]) + "<CR>"
                if r.random() < 0.3:
                    keys += r.choice(["dd", "x", "<C-o>", "''"])
            else:
                keys = self.change() + r.choice([":u<CR>", ":undo<CR>", ":u<CR>:red<CR>", ":u<CR>:redo<CR>"])
            self.add(self.cur, keys)

    def cat_insert(self, n):
        r = self.r
        pieces = ["ab", "x y", "foo bar baz", " ", "<CR>", "<BS>", "<BS><BS>", "<C-w>", "<C-u>", "<C-h>", "word",
                  "é", "日本", "a.b,c", "<C-w><C-w>", "one two", "<Del>", "<Left>", "<Right>", "<C-r>\"", "<C-r>0",
                  "<C-o>x", "<C-o>0", "<C-o>$", "<C-o>dw", "<C-o>D", "<C-o>h", "<C-o>w"]
        for _ in range(n):
            self.buf()
            pre = ""
            if r.random() < 0.3:
                pre = self.yank() if r.random() < 0.5 else self.cmotion()
            start = self.count(0.25) + r.choice(["i", "a", "I", "A", "o", "O", "gI", "R", "i", "a", "cw", "cc", "s", "C", "ciw"])
            body = "".join(r.choice(pieces) for _ in range(r.randrange(1, 5)))
            keys = pre + start + body + "<Esc>"
            k = r.random()
            if k < 0.25:
                keys += self.cmotion() + self.count(0.2) + "."
            elif k < 0.4:
                keys += "u"
            elif k < 0.5:
                keys += self.cmotion() + "gi" + self.text() + "<Esc>"
            elif k < 0.6:
                keys += self.cmotion() + '".p'
            self.add(self.cur, keys)

    def cat_marks(self, n):
        r = self.r
        for _ in range(n):
            self.buf(["prose", "repeat", "paras", "words", "sentences", "sentences2"])
            k = r.random()
            if k < 0.5:
                keys = "ma" + self.cmotion() + (self.cmotion() if r.random() < 0.5 else "")
                keys += r.choice(["'a", "`a", "d'a", "d`a", "y'a", "y`a", "c`aX<Esc>", "v`ad", "V'ad", "mb'a`b", "''", "``", "'a''", "`a``"])
            elif k < 0.8:
                jumps = ["G", "gg", "}", "{", "3G", "/the<CR>", "n", "*", "%", "2G", "5G"] + (["(", ")"] if self.safe() else [])
                keys = "".join(r.choice(jumps) for _ in range(r.randrange(1, 4)))
                keys += "".join(r.choice(["<C-o>", "<C-o>", "<C-i>", "''", "``"]) for _ in range(r.randrange(1, 4)))
            else:
                change = self.change()
                marks = ["`[", "`]", "'[", "']", "`^", "gi!<Esc>"]
                if "<Esc>" not in change:
                    # Typed ahead, Vim inserts the text in one piece and
                    # leaves `. at its start; typed by hand, on the last
                    # character, which is what the engine does.
                    marks += ["`.", "'."]
                keys = change + self.cmotion() + r.choice(marks)
            self.add(self.cur, keys)

    def cat_soup(self, n):
        r = self.r
        for _ in range(n):
            self.buf()
            keys = ""
            for _ in range(r.randrange(3, 8)):
                k = r.random()
                if k < 0.35:
                    keys += self.cmotion()
                elif k < 0.7:
                    keys += self.change()
                elif k < 0.8:
                    keys += self.yank()
                elif k < 0.9:
                    keys += r.choice(["u", "<C-r>", ".", "2.", "p", "P", "gv", "<Esc>"])
                else:
                    op = r.choice(["d", "y", "~", "c"])
                    keys += r.choice(["v", "V"]) + self.cmotion() + op + self.finish_op(op)
            self.add(self.cur, keys)

    CATEGORIES = [
        ("motions", 2.0), ("op_motion", 3.0), ("textobj", 2.0), ("simple", 1.0),
        ("put", 1.5), ("registers", 1.0), ("dot", 1.5), ("undo", 1.5),
        ("visual", 2.0), ("search", 1.0), ("ex", 1.0), ("insert", 1.5),
        ("marks", 0.7), ("soup", 1.5),
    ]

    def all(self, per):
        for name, weight in self.CATEGORIES:
            getattr(self, "cat_" + name)(int(per * weight))


# ----- running Vim ------------------------------------------------------------------

SETTINGS = (
    "set nocompatible encoding=utf-8 nojoinspaces ignorecase smartcase nowrap "
    "backspace=indent,eol,start noautoindent nosmartindent textwidth=0 formatoptions= "
    "nohlsearch noincsearch wrapscan selection=inclusive shiftwidth=4 "
    "matchpairs=(:),{:},[:] whichwrap=b,s nomodeline noswapfile undolevels=1000 "
    "columns=200 lines=50 belloff=all"
)

NOOP_PROBE = """let noop = 0
for i in range(300)
  let n = changenr()
  silent! redo
  if changenr() == n | break | endif
endfor
for i in range(300)
  if changenr() == 0 | break | endif
  let before = getline(1, '$')
  silent! undo
  if getline(1, '$') ==# before | let noop = 1 | endif
endfor
"""

SPECIAL = re.compile(r"<(lt|[A-Za-z]+|[CSAMcsam]-[^>]+)>")


def vim_keys(keys):
    """Harness notation to the inside of a Vim double-quoted string."""
    out = []
    i = 0
    while i < len(keys):
        m = SPECIAL.match(keys, i)
        if m:
            name = m.group(1)
            out.append("<" if name == "lt" else "\\<" + name + ">")
            i = m.end()
            continue
        ch = keys[i]
        if ch in '\\"':
            out.append("\\" + ch)
        else:
            out.append(ch)
        i += 1
    return "".join(out)


def run_case(vim, case):
    lines = BUFFERS[case["buf"]] if "lines" not in case else case["lines"]
    with tempfile.TemporaryDirectory(prefix="vimgen") as d:
        buf = os.path.join(d, "buffer.txt")
        script = os.path.join(d, "case.vim")
        out = os.path.join(d, "out.json")
        with open(buf, "w", encoding="utf-8") as f:
            if lines != [""]:
                f.write("\n".join(lines) + "\n")
        regs = "[" + ",".join("'" + r.replace("'", "''") + "'" for r in REGISTERS) + "]"
        with open(script, "w", encoding="utf-8") as f:
            f.write(SETTINGS + "\n")
            f.write("scriptencoding utf-8\n")
            # Loading the file set marks, and batch mode starts on the last
            # line. Start as the engine does after `attach`: no marks but the
            # context mark at the start of the document, an empty jump list.
            f.write("call cursor(1, 1)\n")
            f.write("normal! m'\n")
            f.write("normal! m'\n")
            f.write("silent! delmarks [ ] . ^ < >\n")
            f.write("clearjumps\n")
            f.write("call cursor(%d, %d)\n" % (case["l"], case["c"]))
            f.write('try\n  call feedkeys("%s\\<Esc>", "tx")\ncatch\nendtry\n' % vim_keys(case["keys"]))
            f.write("let p = getcurpos()\n")
            f.write("let regs = map(%s, {_, r -> [getregtype(r), getreg(r, 1, 1)]})\n" % regs)
            f.write("let res = {'lines': getline(1, '$'), 'l': p[1], 'c': p[2], 'regs': regs}\n")
            # Does the undo history hold a step that changes nothing (a
            # command that left the text as it was)? The engine has no such
            # steps, so undo then goes one change further than in Vim.
            f.write(NOOP_PROBE)
            f.write("let res.noop = noop\n")
            f.write("call writefile([json_encode(res)], '%s')\n" % out)
            f.write("qa!\n")
        try:
            subprocess.run(
                [vim, "-u", "NONE", "-i", "NONE", "-N", "-n", "-es", "-S", script, buf],
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                timeout=10, cwd=d,
            )
            with open(out, encoding="utf-8") as f:
                res = json.loads(f.read())
        except Exception:
            return None
    return res


def esc(s):
    return s.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def esc_lines(lines):
    return "\\n".join(esc(l) for l in lines)


def fixture_line(case, res):
    lines = BUFFERS[case["buf"]] if "lines" not in case else case["lines"]
    fields = [esc_lines(lines), str(case["l"]), str(case["c"]), esc(case["keys"]),
              esc_lines(res["lines"]), str(res["l"]), str(res["c"])]
    for typ, reg_lines in res["regs"]:
        typ = typ[:1]
        if typ not in ("v", "V"):
            typ = ""
        text = "\n".join(reg_lines)
        if typ == "V":
            text += "\n"
        if typ == "" and text:
            # Blockwise registers do not exist in the engine.
            return None
        fields.append(typ)
        fields.append(esc(text))
    tags = []
    if res.get("noop"):
        tags.append("noop-undo")
    fields.append(",".join(tags))
    return "\t".join(fields)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--per-category", type=int, default=300)
    ap.add_argument("--seed", type=int, default=20261111)
    ap.add_argument("--vim", default="/usr/bin/vim")
    ap.add_argument("--keys-file")
    ap.add_argument("--jobs", type=int, default=8)
    args = ap.parse_args()

    g = Gen(args.seed)
    if args.keys_file:
        with open(args.keys_file, encoding="utf-8") as f:
            for raw in f:
                raw = raw.rstrip("\n")
                if not raw or raw.startswith("#"):
                    continue
                name, l, c, keys = raw.split("\t")
                g.add(name, keys, (int(l), int(c)))
    else:
        g.all(args.per_category)

    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda c: run_case(args.vim, c), g.cases))

    written = 0
    with open(args.out, "w", encoding="utf-8") as f:
        for case, res in zip(g.cases, results):
            if res is None:
                continue
            line = fixture_line(case, res)
            if line is None:
                continue
            f.write(line + "\n")
            written += 1
    print("%d cases generated, %d written" % (len(g.cases), written), file=sys.stderr)


if __name__ == "__main__":
    main()
