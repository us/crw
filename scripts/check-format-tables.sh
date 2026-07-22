#!/usr/bin/env bash
# check-format-tables.sh — Snapshot guard for OutputFormat and RequestedRenderer.
#
# Extracts the canonical enum variants from the Rust source of truth
# (crates/crw-core/src/types.rs) and asserts that the docs tables in
# docs/docs/output-formats.md and docs/docs/js-rendering.md list exactly those
# values. Fails loudly on any drift so a new variant or a rename is caught at
# PR time instead of silently shipping wrong docs.
#
# Portable: bash + standard POSIX tools (grep, sort, comm). Works on
# ubuntu-latest and macOS without extra dependencies.
#
# Usage:
#   bash scripts/check-format-tables.sh          # from repo root
#   CHECK_REPO_ROOT=/path/to/repo bash scripts/check-format-tables.sh

set -euo pipefail

cd "${CHECK_REPO_ROOT:-$(dirname "$0")/..}"

TYPES_RS="crates/crw-core/src/types.rs"
OUTPUT_FORMATS_DOC="docs/docs/output-formats.md"
JS_RENDERING_DOC="docs/docs/js-rendering.md"
OPENAPI_SPEC="docs/openapi.json"

FAIL=0

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Print a sorted, newline-separated list to stdout. Usage: sorted_lines <list>
sorted_lines() {
  printf '%s\n' "$@" | sort
}

# Diff two sorted lists (each one item per line via process substitution) and
# print a human-readable report. Returns 1 if they differ.
diff_sets() {
  local label="$1"
  local source_list="$2"   # newline-separated
  local doc_list="$3"       # newline-separated

  local only_in_source only_in_doc
  only_in_source=$(comm -23 \
    <(printf '%s\n' $source_list | sort) \
    <(printf '%s\n' $doc_list   | sort))
  only_in_doc=$(comm -13 \
    <(printf '%s\n' $source_list | sort) \
    <(printf '%s\n' $doc_list   | sort))

  if [ -z "$only_in_source" ] && [ -z "$only_in_doc" ]; then
    echo "ok: ${label} — all variants present and no extras"
    return 0
  fi

  echo "FAIL: ${label} — docs table is out of sync with Rust source" >&2
  if [ -n "$only_in_source" ]; then
    echo "  Missing from docs (add these):" >&2
    printf '    %s\n' $only_in_source >&2
  fi
  if [ -n "$only_in_doc" ]; then
    echo "  Extra in docs (remove or update these):" >&2
    printf '    %s\n' $only_in_doc >&2
  fi
  echo >&2
  return 1
}

# ---------------------------------------------------------------------------
# Verify input files exist
# ---------------------------------------------------------------------------

for f in "$TYPES_RS" "$OUTPUT_FORMATS_DOC" "$JS_RENDERING_DOC"; do
  if [ ! -f "$f" ]; then
    echo "FAIL: required file not found: $f" >&2
    exit 1
  fi
done

# ---------------------------------------------------------------------------
# 1. Extract OutputFormat variants from Rust source
#
# Strategy: find the `pub enum OutputFormat {` block, read until the closing
# brace, pull bare variant names (lines that are just `Identifier,` or
# `Identifier`). This is intentionally conservative — multi-line attributes
# (#[...]) are skipped, keeping only the variant names.
# ---------------------------------------------------------------------------

echo "==> Parsing OutputFormat variants from ${TYPES_RS}"

# Collect lines inside the OutputFormat enum block. Stop at the first bare `}`
# that closes the enum (not a nested struct or match arm).
output_format_variants=()
in_block=0
while IFS= read -r line; do
  if echo "$line" | grep -qE '^pub enum OutputFormat[[:space:]]*\{'; then
    in_block=1
    continue
  fi
  if [ "$in_block" -eq 1 ]; then
    # Closing brace of the enum
    if echo "$line" | grep -qE '^[[:space:]]*\}[[:space:]]*$'; then
      in_block=0
      break
    fi
    # Skip attribute lines (#[...]) and blank lines
    if echo "$line" | grep -qE '^[[:space:]]*(#\[|//)'; then
      continue
    fi
    if echo "$line" | grep -qE '^[[:space:]]*$'; then
      continue
    fi
    # Extract variant name: strip leading whitespace and trailing comma/brace
    variant=$(echo "$line" | sed 's/^[[:space:]]*//' | sed 's/[,{].*//' | tr -d ' ')
    if [ -n "$variant" ]; then
      output_format_variants+=("$variant")
    fi
  fi
done < "$TYPES_RS"

if [ "${#output_format_variants[@]}" -eq 0 ]; then
  echo "FAIL: could not extract any OutputFormat variants from ${TYPES_RS}" >&2
  echo "      Check that the enum is still named 'pub enum OutputFormat'." >&2
  exit 1
fi

echo "  Source variants (${#output_format_variants[@]}): ${output_format_variants[*]}"

# Map Rust PascalCase variant names to their canonical wire keys (camelCase),
# matching serde(rename_all = "camelCase") on OutputFormat. The mapping must
# mirror parse_loose() in types.rs — when a variant is renamed in serde, update
# this table.
#
# PascalCase → camelCase rules applied here:
#   Markdown       → markdown
#   Html           → html
#   RawHtml        → rawHtml
#   PlainText      → plainText
#   Links          → links
#   Json           → json
#   Summary        → summary
#   ChangeTracking → changeTracking
pascal_to_camel() {
  local name="$1"
  # Lower-case the first character; leave the rest as-is (serde camelCase for
  # two-word variants like RawHtml/PlainText/ChangeTracking keeps their
  # internal uppercase).
  printf '%s' "${name:0:1}" | tr '[:upper:]' '[:lower:]'
  printf '%s' "${name:1}"
}

output_format_keys=()
for v in "${output_format_variants[@]}"; do
  output_format_keys+=("$(pascal_to_camel "$v")")
done

echo "  Wire keys: ${output_format_keys[*]}"

# ---------------------------------------------------------------------------
# 2. Extract RequestedRenderer variants from Rust source
#
# RequestedRenderer uses serde(rename_all = "lowercase") with one explicit
# override: ChromeProxy → "chrome_proxy". We apply those same rules.
# ---------------------------------------------------------------------------

echo "==> Parsing RequestedRenderer variants from ${TYPES_RS}"

# Parse the RequestedRenderer enum block manually using pure bash + POSIX tools.
# We need to handle: doc comments (///), attributes (#[...]), and one explicit
# serde rename (#[serde(rename = "chrome_proxy")]) that overrides the
# rename_all = "lowercase" default.
renderer_keys_raw=""
in_block=0
pending_rename=""
while IFS= read -r line; do
  # Trim leading whitespace for matching
  trimmed=$(printf '%s' "$line" | sed 's/^[[:space:]]*//')

  if printf '%s' "$trimmed" | grep -qE '^pub enum RequestedRenderer[[:space:]]*\{'; then
    in_block=1
    continue
  fi

  if [ "$in_block" -eq 1 ]; then
    # Closing brace ends the enum
    if printf '%s' "$trimmed" | grep -qE '^\}[[:space:]]*$'; then
      in_block=0
      break
    fi

    # Serde rename attribute — capture the quoted value
    if printf '%s' "$trimmed" | grep -qE '^#\[serde\(rename[[:space:]]*='; then
      pending_rename=$(printf '%s' "$trimmed" \
        | sed 's/.*rename[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/')
      continue
    fi

    # Skip other attributes, doc comments, blank lines
    if printf '%s' "$trimmed" | grep -qE '^(#\[|//|$)'; then
      continue
    fi

    # Extract the variant name (up to first comma, brace, or space)
    variant=$(printf '%s' "$trimmed" | sed 's/[,{ ].*//')
    [ -z "$variant" ] && continue

    if [ -n "$pending_rename" ]; then
      renderer_keys_raw="${renderer_keys_raw}${pending_rename}"$'\n'
      pending_rename=""
    else
      renderer_keys_raw="${renderer_keys_raw}$(printf '%s' "$variant" | tr '[:upper:]' '[:lower:]')"$'\n'
    fi
  fi
done < "$TYPES_RS"

if [ -z "$renderer_keys_raw" ]; then
  echo "FAIL: could not extract any RequestedRenderer variants from ${TYPES_RS}" >&2
  exit 1
fi

# Convert newline-separated string to array (portable — no mapfile/readarray).
renderer_keys=()
while IFS= read -r k; do
  [ -n "$k" ] && renderer_keys+=("$k")
done <<< "$renderer_keys_raw"

echo "  Wire keys: ${renderer_keys[*]}"

# ---------------------------------------------------------------------------
# 3. Check output-formats.md table lists exactly the OutputFormat wire keys
#
# The doc table uses backtick-quoted keys like `markdown`, `rawHtml`, etc.
# We extract the second column (| `key` |) from the Formats table only — we
# look for lines that contain a backtick-quoted token in a Markdown table cell
# in the ## Formats section.
#
# Aliases (extract, llm-extract, change-tracking) are intentionally listed in
# the doc but are NOT enum variants — we exclude them from the source-side
# comparison. They appear explicitly in a dedicated "Format Aliases" section.
# ---------------------------------------------------------------------------

echo "==> Checking ${OUTPUT_FORMATS_DOC}"

# Extract `backtick` tokens from table rows in the Formats section only.
# We read from "## Formats" down to the next "##" heading.
doc_output_keys=()
in_formats=0
while IFS= read -r line; do
  if echo "$line" | grep -qE '^## Formats[[:space:]]*$'; then
    in_formats=1
    continue
  fi
  if [ "$in_formats" -eq 1 ] && echo "$line" | grep -qE '^## '; then
    # Reached the next section — stop
    break
  fi
  if [ "$in_formats" -eq 1 ]; then
    # Match table rows of the form: | Label | `key` | Description |
    # The key column is the 2nd pipe-delimited cell. Extract backtick tokens.
    if echo "$line" | grep -qE '^\|'; then
      # Grab the first backtick-quoted token in the table row.
      # Use || true to avoid pipefail exit when grep finds no match (header/
      # separator rows have no backtick keys).
      token=$(echo "$line" | grep -oE '`[a-zA-Z][a-zA-Z0-9_-]*`' | head -1 | tr -d '`' || true)
      if [ -n "$token" ]; then
        doc_output_keys+=("$token")
      fi
    fi
  fi
done < "$OUTPUT_FORMATS_DOC"

# The doc intentionally lists "Extract" as an alias row (key: `extract`).
# Strip aliases that are NOT canonical enum variant wire keys so the comparison
# is fair.  Aliases: extract, llm-extract, change-tracking.
ALIASES=(extract llm-extract change-tracking)
filtered_doc_output_keys=()
for k in "${doc_output_keys[@]}"; do
  is_alias=0
  for a in "${ALIASES[@]}"; do
    if [ "$k" = "$a" ]; then
      is_alias=1
      break
    fi
  done
  if [ "$is_alias" -eq 0 ]; then
    filtered_doc_output_keys+=("$k")
  fi
done

echo "  Doc keys (after alias strip): ${filtered_doc_output_keys[*]}"

if [ "${#filtered_doc_output_keys[@]}" -eq 0 ]; then
  echo "FAIL: no keys extracted from the Formats table in ${OUTPUT_FORMATS_DOC}" >&2
  echo "      Check that the '## Formats' section and its Markdown table still exist." >&2
  FAIL=1
else
  if ! diff_sets "OutputFormat (output-formats.md)" \
       "${output_format_keys[*]}" \
       "${filtered_doc_output_keys[*]}"; then
    FAIL=1
  fi
fi

# ---------------------------------------------------------------------------
# 4. Check js-rendering.md table lists exactly the RequestedRenderer wire keys
#
# The renderer table is under "## Per-request renderer override". We look for
# the first backtick token per table row there.
# ---------------------------------------------------------------------------

echo "==> Checking ${JS_RENDERING_DOC}"

doc_renderer_keys=()
in_renderer=0
while IFS= read -r line; do
  if echo "$line" | grep -qE '^## Per-request renderer override[[:space:]]*$'; then
    in_renderer=1
    continue
  fi
  if [ "$in_renderer" -eq 1 ] && echo "$line" | grep -qE '^## '; then
    break
  fi
  if [ "$in_renderer" -eq 1 ]; then
    if echo "$line" | grep -qE '^\|'; then
      token=$(echo "$line" | grep -oE '`[a-zA-Z][a-zA-Z0-9_-]*`' | head -1 | tr -d '`' || true)
      if [ -n "$token" ]; then
        doc_renderer_keys+=("$token")
      fi
    fi
  fi
done < "$JS_RENDERING_DOC"

# The doc table has a header-separator row that produces an empty token — it is
# already filtered by the regex requiring [a-zA-Z] at start. The "omitted /
# auto" combined row counts as "auto" (first token). That is fine — auto is a
# valid variant key.

echo "  Doc keys: ${doc_renderer_keys[*]}"

if [ "${#doc_renderer_keys[@]}" -eq 0 ]; then
  echo "FAIL: no keys extracted from the renderer override table in ${JS_RENDERING_DOC}" >&2
  echo "      Check that '## Per-request renderer override' and its Markdown table still exist." >&2
  FAIL=1
else
  if ! diff_sets "RequestedRenderer (js-rendering.md)" \
       "${renderer_keys[*]}" \
       "${doc_renderer_keys[*]}"; then
    FAIL=1
  fi
fi

# ---------------------------------------------------------------------------

if [ "$FAIL" -ne 0 ]; then
  echo >&2
  echo "FAIL: format/renderer table drift detected." >&2
  echo "      Update the docs tables to match crates/crw-core/src/types.rs." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 5. Check the OpenAPI spec's ScrapeRequest.formats enum lists exactly the
#    OutputFormat wire keys.
#
#    This is the gap that let `screenshot` and `images` sit in the enum in
#    types.rs while the published spec said they did not exist, for long enough
#    that our own marketing copied the omission as a missing feature.
#    `check-openapi.sh` cannot catch it: it only compares the committed spec
#    against the copy the binary embeds, which is the same file.
#
#    Aliases are stripped for the same reason as the markdown tables: they are
#    documented in the property description, not as enum members.
# ---------------------------------------------------------------------------

echo "==> Checking ${OPENAPI_SPEC} OutputFormat enum"

if [ ! -f "$OPENAPI_SPEC" ]; then
  echo "FAIL: ${OPENAPI_SPEC} not found" >&2
  FAIL=1
else
  # Reads the single `OutputFormat` component and asserts every ScrapeRequest-
  # style formats array points at it by $ref rather than inlining a copy. An
  # inlined enum is how `screenshot` and `images` could sit in types.rs while
  # the published spec said they did not exist — long enough that our own
  # marketing copied the omission as a missing feature.
  #
  # `check-openapi.sh` cannot catch this: it only compares the committed spec
  # against the copy the binary embeds, which is the same bytes.
  spec_format_keys_raw=$(python3 - "$OPENAPI_SPEC" 2>&1 <<'PYEOF'
import json, sys

spec = json.load(open(sys.argv[1]))
schemas = spec["components"]["schemas"]
enum = schemas.get("OutputFormat", {}).get("enum")
if not enum:
    sys.exit("components.schemas.OutputFormat has no enum")

# Every formats array that is meant to carry the full set must $ref it.
REF = "#/components/schemas/OutputFormat"
for name in ("ScrapeRequest", "BatchScrapeRequest"):
    items = schemas[name]["properties"]["formats"].get("items", {})
    if items.get("$ref") != REF:
        sys.exit(
            f"{name}.formats.items must be a $ref to OutputFormat, not an inline "
            f"copy (found: {items})"
        )

print("\n".join(enum))
PYEOF
  ) || {
    echo "FAIL: ${OPENAPI_SPEC} formats check failed: ${spec_format_keys_raw}" >&2
    FAIL=1
    spec_format_keys_raw=""
  }

  spec_format_keys=()
  while IFS= read -r k; do
    [ -n "$k" ] && spec_format_keys+=("$k")
  done <<< "$spec_format_keys_raw"

  if [ "${#spec_format_keys[@]}" -eq 0 ]; then
    FAIL=1
  else
    echo "  Spec enum: ${spec_format_keys[*]}"
    if ! diff_sets "OutputFormat (openapi.json)" \
         "${output_format_keys[*]}" \
         "${spec_format_keys[*]}"; then
      echo "      The spec is what SDKs and agents read. A format missing here" >&2
      echo "      reads as unsupported even when the engine implements it." >&2
      FAIL=1
    fi
  fi
fi

# The crate copy is what the binary serves; it must stay byte-identical.
echo "==> Checking the embedded spec copy matches ${OPENAPI_SPEC}"
if ! diff -q "$OPENAPI_SPEC" crates/crw-server/openapi/openapi.json >/dev/null; then
  echo "FAIL: docs/openapi.json and crates/crw-server/openapi/openapi.json differ." >&2
  echo "      The binary embeds the crate copy; they must be identical." >&2
  FAIL=1
fi

if [ "$FAIL" -ne 0 ]; then
  echo >&2
  echo "FAIL: format/renderer table drift detected." >&2
  echo "      Update the docs tables to match crates/crw-core/src/types.rs." >&2
  exit 1
fi

echo "All format/renderer snapshot checks passed."
