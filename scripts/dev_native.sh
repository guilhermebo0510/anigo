#!/bin/bash
# Fluxo "editar → ver" do app nativo.
# Uso: scripts/dev_native.sh [studio|test-model|check]
# Requer: cargo install bacon   (ou use `cargo run -p anigo-studio` direto)
set -euo pipefail
JOB=${1:-studio}
if command -v bacon >/dev/null 2>&1; then
  exec bacon "$JOB"
fi
echo "bacon não encontrado; rodando uma vez (cargo install bacon para o modo watch)"
case "$JOB" in
  studio) exec cargo run -p anigo-studio -- --session-file target/dev_session.json ;;
  test-model) exec cargo test -p anigo-modeling ;;
  check) exec cargo check --workspace --all-targets ;;
esac
