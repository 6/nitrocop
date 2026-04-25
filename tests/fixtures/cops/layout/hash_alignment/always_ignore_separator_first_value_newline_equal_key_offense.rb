# nitrocop-config: EnforcedStyle: separator, separator, always_ignore
# RuboCop 1.84.2 still emits offenses when newline-value separator corrections
# can be applied without overlapping the value indentation, so we must too.
hash = {
  foo:
    bar.qux,
  baz: name
  ^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
}

resource = {
  foo:
    if cond
      x
    end,
  longer: name
  ^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
}

absorbed = {
  "long":
    first,
  "a":
  ^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
    second
}

first_not_widest = {
  "medium":
    first,
  "a much longer key":
  ^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
    second,
  "a":
  ^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
    third
}
