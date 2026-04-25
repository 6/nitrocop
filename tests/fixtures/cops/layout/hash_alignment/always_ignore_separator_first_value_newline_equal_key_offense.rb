# nitrocop-config: EnforcedStyle: separator, separator, always_ignore
# RuboCop 1.84.2 only crashes the separator-style autocorrect when a colon
# pair's key is *strictly shorter* than the first pair's. When subsequent
# keys are equal length or longer, it still emits offenses, so we must too.
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
