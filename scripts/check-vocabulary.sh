#!/usr/bin/env bash
#
# The glossary's avoided words, checked where a machine can decide.
#
# GLOSSARY.md gives each term an `_Avoid_:` list. Almost all of those lists are semantic: they
# forbid a word when it means that concept — `state` for Outcome, `recording` for Dictation, `pill`
# for Presenter — and the same word is right everywhere else, so no grep can settle them. Clipped's
# is the exception: `saturated`, `distorted` and `overloaded` name nothing else here, and it is the
# list that was breached. The term, its entry and two ADRs calling a clipped recording "saturated"
# all landed in the same session, and only a reading caught it.
#
# Run from the repository root. Tracked files are read from the index; the glossary is skipped
# because it is where the words are written down, and so is this script.

set -euo pipefail

readonly AVOIDED='saturated|distorted|overloaded'
readonly SELF='scripts/check-vocabulary.sh'

found=0
while IFS= read -r file; do
    case "$file" in
        GLOSSARY.md | "$SELF") continue ;;
    esac

    if matches=$(grep -nEi "$AVOIDED" "$file"); then
        printf '%s\n' "$matches" | sed "s|^|$file:|"
        found=1
    fi
done < <(git ls-files '*.md' '*.rs' '*.sh' '*.toml' '*.yml' '*.jsonc' 'transcribers/*')

if [ "$found" -ne 0 ]; then
    printf '\nThis repository says "clipped", never %s (GLOSSARY.md, Clipped).\n' "$AVOIDED" >&2
    exit 1
fi
