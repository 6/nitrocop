# nitrocop-config: EnforcedStyle: require_no_parentheses_except_multiline

# Single-line args without parens: OK
def foo x, y
  x + y
end

# No args: OK
def bar
  42
end

# Multiline args with parens: OK (parens required for multiline)
def qux(x,
        y)
  x + y
end

# Multiline args with parens where params are on a separate line from parens: OK
# (ParametersNode covers only a single line, but lparen..rparen spans multiple)
def initialize(
  enabled:
)
  @enabled = enabled
end

# Multiline args with parens, keyword with default on separate line: OK
def setup(
  logger: Rails.logger
)
  @logger = logger
end

# Multiline args with parens, multiple keyword args on same line: OK
def create(
  project_id:, instance_id:, database_id:, kms_key_names:
)
  do_something
end

# Multiline args with parens, two keyword args across lines: OK
def build(
  scaffold:, name:

)
  do_something
end

# Endless method with parens: OK (forced parens)
def baz(x) = x + 1

# Forwarding parameter: OK (forced parens)
def fwd(...)
  other(...)
end

# Anonymous rest: OK (forced parens)
def anon_rest(*)
end

# Anonymous kwrest: OK (forced parens)
def anon_kwrest(**)
end

# Anonymous block: OK (forced parens)
def anon_block(&)
  yield
end

# Empty multiline parens: OK (RuboCop considers the args node multiline
# when lparen..rparen spans lines, even with no actual parameters)
def empty_multiline(
)
end
