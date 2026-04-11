# nitrocop-config: EnforcedStyle: non_integer

# Both normalcase and snake_case trailing digits are offenses under non_integer
foo1 = 1
^^^^ Naming/VariableNumber: Use non_integer for variable numbers.
foo_1 = 1
^^^^^ Naming/VariableNumber: Use non_integer for variable numbers.

def some_method1; end
    ^^^^^^^^^^^^ Naming/VariableNumber: Use non_integer for method name numbers.

def some_method_1; end
    ^^^^^^^^^^^^^^ Naming/VariableNumber: Use non_integer for method name numbers.

:some_sym1
 ^^^^^^^^^ Naming/VariableNumber: Use non_integer for symbol numbers.

:some_sym_1
 ^^^^^^^^^^ Naming/VariableNumber: Use non_integer for symbol numbers.

# FN fix: windows-1251 encoding with ASCII-only content is processable
# (RuboCop's Translation::Parser handles ASCII bytes in any encoding)
def test_windows_1251; end
    ^^^^^^^^^^^^^^^^^ Naming/VariableNumber: Use non_integer for method name numbers.

c1 = 1
^^ Naming/VariableNumber: Use non_integer for variable numbers.

c2 = 2
^^ Naming/VariableNumber: Use non_integer for variable numbers.
