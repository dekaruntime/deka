# PHPX Internal Bridge Audit

Goal: minimize __deka_* usage. Keep only truly low-level hooks and migrate
everything else into phpx.

## JS emitter builtin rewrites
PHPX keeps PHP-shaped surface calls, but the JS backend lowers common builtins
to native JS at compile time instead of installing PHP-named globals. Examples:

- `count($a)` emits `$a.length` for known arrays/strings, `Object.keys($a).length`
  for known objects, or a single-evaluation inline fallback when the value kind
  is unknown.
- `strlen($s)` emits `$s.length`.
- `substr($s, $i, $n)` emits `$s.slice($i, $i + $n)`.
- `array_keys($a)` / `array_values($a)` emit `Object.keys($a)` /
  `Object.values($a)`.
- `in_array($needle, $haystack)` emits `$haystack.includes($needle)`.
- `array_map($fn, $a)` and `array_filter($a, $fn)` emit `.map` / `.filter`.
- `is_array($x)` emits `Array.isArray($x)`; structs are objects, so the native
  array check excludes them.

If a builtin genuinely needs reusable runtime logic, the emitter records a
tree-shakable module-scoped helper named `__phpx_X`. Do not add
`globalThis.<php_name>` polyfills for PHP-named surface builtins.

## PHPX Syntax Additions (recent)
### Type aliases (phpx only)
Syntax:
- `type Person = Object<{ id: int, name: string, email?: string }>;`
- `type Person = { id: int, name: string, email?: string };` (object-shape sugar)

Behavior:
- Compile-time only; stripped during emission.
- Aliases can reference any type expression (including unions, nullable, structs).
- Object-shape rules apply (exact fields, optional `?`, dot access yields `T|null`).
- Aliases are top-level only and must not shadow built-in types or structs.

## Keep (Rust-only, for now)
- chr, ord (byte primitives)
- crc32, md5, md5_file, sha1, sha1_file, crypt (hash/crypto)

## Keep until phpx can access VM state directly
- __deka_symbol_get, __deka_symbol_set, __deka_symbol_exists (symbol table access)
- __deka_array_cursor (array internal pointer state)

## Candidates to reimplement in phpx (remove __deka_* usage)
### String
- [x] convert_cyr_string
- [x] convert_uuencode
- [x] convert_uudecode
- [x] metaphone
- [x] number_format
- [x] strcmp
- [x] strcasecmp
- [x] strncmp
- [x] strncasecmp
- [x] strnatcmp
- [x] strnatcasecmp
- [x] levenshtein
- [x] similar_text
- [x] soundex
- [x] substr_compare
- [x] strstr
- [x] stristr
- [x] hebrev
- [x] wordwrap
- [x] quotemeta
- [x] nl2br
- [x] strip_tags
- [x] strtok
- [x] count_chars
- [x] str_word_count
- [x] str_increment
- [x] str_decrement
- [x] htmlspecialchars
- [x] htmlspecialchars_decode
- [x] htmlentities
- [x] html_entity_decode
- [x] str_replace
- [x] str_ireplace
- [x] utf8_encode
- [x] utf8_decode
- [x] version_compare
- [x] setlocale
- [x] localeconv
- [x] nl_langinfo
- [x] strcoll
- [x] money_format
- [x] sprintf
- [x] sscanf
- [x] printf
- [x] vsprintf
- [x] vprintf
- [x] fprintf
- [x] vfprintf

### Array
- [x] array
- [x] array_all
- [x] array_any
- [x] array_change_key_case
- [x] array_chunk
- [x] array_column
- [x] array_combine
- [x] array_count_values
- [x] array_diff
- [x] array_diff_assoc
- [x] array_diff_key
- [x] array_diff_uassoc
- [x] array_diff_ukey
- [x] array_fill
- [x] array_fill_keys
- [x] array_filter
- [x] array_find
- [x] array_find_key
- [x] array_first
- [x] array_flip
- [x] array_intersect
- [x] array_intersect_assoc
- [x] array_intersect_key
- [x] array_intersect_uassoc
- [x] array_intersect_ukey
- [x] array_is_list
- [x] array_key_first
- [x] array_key_last
- [x] array_last
- [x] array_map
- [x] array_merge
- [x] array_merge_recursive
- [x] array_multisort
- [x] array_pad
- [x] array_product
- [x] array_reduce
- [x] array_replace
- [x] array_replace_recursive
- [x] array_reverse
- [x] array_search
- [x] array_shift
- [x] array_slice
- [x] array_splice
- [x] array_sum
- [x] array_rand
- [x] array_udiff
- [x] array_udiff_assoc
- [x] array_udiff_uassoc
- [x] array_uintersect
- [x] array_uintersect_assoc
- [x] array_uintersect_uassoc
- [x] array_walk
- [x] array_walk_recursive
- [x] arsort
- [x] asort
- [x] key_exists
- [x] krsort
- [x] natcasesort
- [x] natsort
- [x] range
- [x] rsort
- [x] shuffle
- [x] sort
- [x] uasort
- [x] uksort
- [x] usort
- [x] ksort
