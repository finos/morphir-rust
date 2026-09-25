#!/bin/sh
# A stand-in for `morphir` in morphir-bdd's own tests: prints its arguments, HOME, MORPHIR_HOME
# and whether MORPHIR_BDD_LEAK_PROBE leaked through from the parent environment, exits with
# $EXIT_WITH or 0.
# `fail` as the first argument exits 3, to exercise a failing command.
[ "$1" = fail ] && exit 3
printf '%s\n' "$@"
printf 'home=%s\n' "$HOME"
printf 'morphir_home=%s\n' "$MORPHIR_HOME"
printf 'leak=%s\n' "$MORPHIR_BDD_LEAK_PROBE"
exit "${EXIT_WITH:-0}"
