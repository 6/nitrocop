# nitrocop-config: EnforcedStyle: ignore_explicit
data = {
  :a   => 0,
  ^^^^^^^^^ Layout/HashAlignment: Align the keys of a hash literal if they span more than one line.
  :bb => 1,
}

# An explicit hash followed by `&block` is not the last argument, so RuboCop
# still inspects it under ignore_explicit.
shrine_class.instrument(:derivatives, {
  processor:         processor_name,
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys of a hash literal if they span more than one line.
  processor_options: processor_options,
  io:                source,
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys of a hash literal if they span more than one line.
  attacher:          self,
  ^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the keys of a hash literal if they span more than one line.
}, &block)
