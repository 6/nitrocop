# nitrocop-config: EnforcedStyle: semantic

# fail outside rescue → OK (signaling)
fail RuntimeError, "message"
fail "something went wrong"
fail

# raise inside rescue body → OK (rethrowing)
begin
  fail
rescue Exception
  raise RuntimeError
end

# raise in def rescue body → OK
def test
  fail
rescue Exception
  raise
end

# Explicit receiver (not Kernel) → OK
test.raise
test.fail

# raise in multiple rescue bodies → OK
def test
  fail
rescue StandardError
  raise
rescue Exception
  raise
end

# fail in begin body and raise in rescue → OK (correct semantic usage)
begin
  fail "something"
rescue StandardError
  raise
end
