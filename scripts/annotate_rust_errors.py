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

MAX_BLOCKS = 60
MAX_BLOCK_LINES = 22
# O GitHub trunca a mensagem de uma anotação em 4096 caracteres e aceita até 10
# anotações por passo: os blocos são distribuídos em várias anotações para que o
# log inteiro caiba (40 KB) em vez de só os primeiros 4 KB.
ANNOTATION_CHARS = 3800
# Bloco condensado de um teste quebrado: um por anotação, com folga para a
# mensagem inteira (um dump de `SkinAssignmentSummary`/`ProjectState` cortado no
# meio esconde justamente o campo que explica a falha).
TEST_ANNOTATION_CHARS = 1500
MAX_ANNOTATIONS = 10

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

    # Falhas de teste não produzem `error[...]`: o cargo imprime os blocos
    # `---- teste stdout ----` e a lista em `failures:`. Isto vira anotação
    # própria (uma por bloco, em várias anotações) para que *todos* os testes
    # quebrados fiquem legíveis na aba Checks — antes só o primeiro aparecia,
    # o que escondia o resto da suíte.
    test_annotations = test_failure_annotations(lines, label)
    for message in test_annotations:
        print(escape(message, prefix="::error::"))

    if not blocks:
        tail = "\n".join(lines[-400:]) or "(log vazio)"
        message = f"{label} falhou; nenhum bloco 'error[...]' reconhecido. Fim do log:\n{tail}"
        print(escape(message, prefix="::error::"))
        return 1

    header = f"{label} falhou ({len(blocks)} bloco(s) de erro no log)"
    chunks: list[list[str]] = [[]]
    size = 0
    for block in blocks:
        text = "\n".join(block)
        if chunks[-1] and size + len(text) > ANNOTATION_CHARS:
            chunks.append([])
            size = 0
        chunks[-1].append(text)
        size += len(text) + 2

    for index, chunk in enumerate(chunks[:MAX_ANNOTATIONS]):
        suffix = f" [{index + 1}/{len(chunks)}]" if len(chunks) > 1 else ""
        message = f"{header}{suffix}:\n\n" + "\n\n".join(chunk)
        print(escape(message, prefix="::error::"))
    if len(chunks) > MAX_ANNOTATIONS:
        print(
            f"::error::{header}: {len(chunks) - MAX_ANNOTATIONS} bloco(s) além do "
            "limite de anotações — veja o log do job"
        )
    return 1


BLOCK_HEADER = re.compile(r"^----\s+.+\s+stdout\s+----$")
# O que interessa de um teste quebrado: onde estourou e a asserção. Os dumps de
# `left:`/`right:` podem trazer um `ProjectState` inteiro (milhares de
# caracteres) e são truncados.
KEEP_LINE = re.compile(r"^(thread '|assertion |panicked at|called `|note: |\s*(left|right):)")


def _truncate(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[: limit - 1] + "…"


def failing_test_blocks(lines: list[str]) -> list[str]:
    """Blocos `---- <teste> stdout ----` condensados (um por teste quebrado)."""
    blocks: list[str] = []
    index = 0
    while index < len(lines) and len(blocks) < MAX_BLOCKS:
        if not BLOCK_HEADER.match(lines[index]):
            index += 1
            continue
        kept = [lines[index].strip()]
        index += 1
        # As **duas** linhas seguintes são o cabeçalho do panic
        # (`panicked at arquivo:linha:coluna:`) e a mensagem da asserção — que
        # não casa com KEEP_LINE e é justamente o diagnóstico (ex.: "osso da
        # cabeça em y = …", o dump de `SkinAssignmentSummary`, o erro de
        # isomerismo). Elas entram sempre.
        for _ in range(2):
            if index < len(lines) and not BLOCK_HEADER.match(lines[index]):
                kept.append(_truncate(lines[index].strip(), 600))
                index += 1
        while index < len(lines) and not BLOCK_HEADER.match(lines[index]):
            if len(kept) < MAX_BLOCK_LINES and KEEP_LINE.match(lines[index]):
                kept.append(_truncate(lines[index].strip(), 600))
            index += 1
        blocks.append("\n".join(kept))
    return blocks


def failing_test_names(lines: list[str]) -> list[str]:
    """Nomes da lista final de `failures:` do cargo (a suíte inteira)."""
    start = None
    for index, line in enumerate(lines):
        if line.strip() == "failures:":
            start = index + 1
    if start is None:
        return []
    names: list[str] = []
    for line in lines[start:]:
        stripped = line.strip()
        if not stripped or stripped.startswith("test result"):
            break
        names.append(stripped)
    return names


def test_failure_annotations(lines: list[str], label: str) -> list[str]:
    """Anotações com *todos* os testes que falharam (nomes + asserção de cada um)."""
    blocks = failing_test_blocks(lines)
    names = failing_test_names(lines)
    if not blocks and not names:
        return []

    header = f"{label}: falhas de teste"
    annotations: list[str] = []
    if names:
        # A lista de nomes é compacta e completa: é o que garante que nenhum
        # teste quebrado fique invisível, mesmo com o limite de anotações.
        annotations.append(f"{header} — {len(names)} teste(s) falharam:\n\n" + "\n".join(names))

    # Um bloco por anotação: nada é cortado pela divisão em pedaços e o limite
    # de 10 anotações do passo cobre oito testes quebrados (mais a lista).
    for index, block in enumerate(blocks):
        if len(annotations) >= MAX_ANNOTATIONS - 1:
            annotations.append(
                f"{header}: {len(blocks) - index} bloco(s) além do limite de "
                f"{MAX_ANNOTATIONS} anotações — veja o log do job"
            )
            break
        annotations.append(f"{header}:\n\n{block[:TEST_ANNOTATION_CHARS]}")

    return annotations[:MAX_ANNOTATIONS]


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
