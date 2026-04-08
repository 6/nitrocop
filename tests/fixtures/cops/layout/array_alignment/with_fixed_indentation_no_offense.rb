# nitrocop-config: EnforcedStyle: with_fixed_indentation

# First element on its own line, properly indented (indent_width from bracket line)
x = [
  1,
  2,
  3]

# First element inline with bracket, other elements at fixed indentation
y = [1,
  2,
  3]

# Rescue exception list at fixed indentation from rescue keyword line
begin
  foo
rescue ArgumentError,
  RuntimeError,
  TypeError => e
  bar
end

# Rescue with line continuation — fixed indentation from rescue keyword
begin
  run_command
rescue \
  FooError,
  BarError => e
  handle
end

# Bracketless array where parent starts on prior line
  config.cache_store =
    :memory_store,
    { size: 128 }
