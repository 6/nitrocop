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

emits_before_clobber = {
  "aaaaaaaaaa":
    first,
  "aaaaaaaa":
  ^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
       second,
  "a":
    third
}

conjur_abort_after_first_offense = {
  "When no signing key properties is set and hash is empty":
    first,
  "When no signing key properties is set and there are fields in hash":
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
    second,
  "When all signing key properties are define":
    third
}
