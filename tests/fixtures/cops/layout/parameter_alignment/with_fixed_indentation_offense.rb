# nitrocop-config: EnforcedStyle: with_fixed_indentation

# with_fixed_indentation: All params on same continuation line
# The first param on a continuation line should be at def_indent + 2
def foo(
    a, b, c)
    ^^^ Layout/ParameterAlignment: Use one level of indentation for parameters following the first line of a multi-line method definition.
  body
end