#!/bin/sh
# A stand-in for `morphir` in morphir-bdd's own tests: prints its arguments and HOME, exits with
# $EXIT_WITH or 0. `fail` as the first argument exits 3, to exercise a failing command.
[ "$1" = fail ] && exit 3
printf '%s\n' "$@"
printf 'home=%s\n' "$HOME"
exit "${EXIT_WITH:-0}"
