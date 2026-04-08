# nitrocop-config: EnforcedStyle: semantic

# Bare raise outside rescue → use fail
raise RuntimeError, "message"
^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.

raise "something went wrong"
^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.

raise
^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.

# raise in begin body → use fail
begin
  raise
  ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.
rescue Exception
  # handle it
end

# raise in def body → use fail
def test
  raise
  ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.
rescue Exception
  # handle it
end

# fail in rescue body → use raise
begin
  fail
rescue Exception
  fail
  ^^^^ Style/SignalException: Use `raise` instead of `fail` to rethrow exceptions.
end

# fail in def rescue body → use raise
def test
  fail
rescue Exception
  fail
  ^^^^ Style/SignalException: Use `raise` instead of `fail` to rethrow exceptions.
end

# fail in second rescue body → use raise
def test
  fail
rescue StandardError
  # handle error
rescue Exception
  fail
  ^^^^ Style/SignalException: Use `raise` instead of `fail` to rethrow exceptions.
end

# raise in block → use fail
map do
  raise 'I'
  ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.
end.flatten.compact

# Kernel.raise outside rescue → use fail
Kernel.raise "error"
       ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.

# Kernel.fail in rescue body → use raise
begin
  fail
rescue Exception
  Kernel.fail "error"
         ^^^^ Style/SignalException: Use `raise` instead of `fail` to rethrow exceptions.
end

# ::Kernel.raise outside rescue → use fail
def test
  ::Kernel.raise
           ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.
rescue Exception
  ::Kernel.fail
           ^^^^ Style/SignalException: Use `raise` instead of `fail` to rethrow exceptions.
end

# Nested begin/rescue
begin
  raise
  ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.
  begin
    raise
    ^^^^^ Style/SignalException: Use `fail` instead of `raise` to signal exceptions.
  rescue
    fail
    ^^^^ Style/SignalException: Use `raise` instead of `fail` to rethrow exceptions.
  end
rescue Exception
  # handle it
end
