#!/usr/bin/env python3
"""Transforma a saída do cargo em **anotações** do check-run.

Por que existe: o log do job vive num blob do GitHub que nem todo ambiente
consegue baixar (`gh api .../jobs/<id>/logs`), mas as anotações do check-run são
legíveis pela API (`/check-runs/<id>/annotations`) e aparecem na aba Checks do
PR. Sem isto, um `cargo check` vermelho no CI é uma caixa-preta: só se sabe que
falhou, não o que falhou.

Uso: python3 scripts/annotate_rust_errors.py <arquivo-de-log> "<rótulo>"

O GitHub aceita no máximo 10 anotações por passo, então os blocos de erro são
concatenados numa anotação só (com escape de `%` e quebras de linha) e o
restante vai no resumo impresso no log.
"""

import re
import sys

MAX_BLOCKS = 40
MAX_BLOCK_LINES = 22
MAX_CHARS = 60000

# O cargo coloriza a saída mesmo em pipeline (o runner não é TTY, mas o cargo
# detecta `TERM`/`CLICOLOR_FORCE`); sem limpar os escapes o `^error[` não casa.
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
HEADER = re.compile(r"^(error|warning)(\[[A-Za-z0-9_]+\])?:")
LOCATION = re.compile(r"^\s*-->")


def annotate(path: str, label: str) -> int:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as handle:
            lines = [ANSI.sub("", line) for line in handle.read().splitlines()]
    except OSError as error:
        print(f"::error::{label}: log ilegível ({error})")
        return 1

    blocks: list[list[str]] = []
    index = 0
    while index < len(lines) and len(blocks) < MAX_BLOCKS:
        if HEADER.match(lines[index]):
            # o bloco vai até o próximo erro/aviso (ou MAX_BLOCK_LINES linhas):
            # inclui o caminho:linha, o trecho de código e as notas do rustc
            end = index + 1
            while (
                end < len(lines)
                and end - index < MAX_BLOCK_LINES
                and not HEADER.match(lines[end])
            ):
                end += 1
            while end > index and not lines[end - 1].strip():
                end -= 1
            blocks.append(lines[index:end])
            index = end
        else:
            index += 1

    if not blocks:
        tail = "\n".join(lines[-400:]) or "(log vazio)"
        message = f"{label} falhou; nenhum bloco 'error[...]' reconhecido. Fim do log:\n{tail}"
        print(escape(message, prefix="::error::"))
        return 1

    body = "\n\n".join("\n".join(block) for block in blocks)
    message = f"{label} falhou ({len(blocks)} bloco(s) de erro no log):\n\n{body}"
    if len(message) > MAX_CHARS:
        message = message[: MAX_CHARS - 40] + "\n... (truncado; veja o log do job)"
    print(escape(message, prefix="::error::"))
    return 1


def escape(message: str, prefix: str) -> str:
    """Escapa uma mensagem de várias linhas para um comando de workflow."""
    return (
        prefix
        + message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    )


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("uso: annotate_rust_errors.py <log> <rótulo>", file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(annotate(sys.argv[1], sys.argv[2]))
