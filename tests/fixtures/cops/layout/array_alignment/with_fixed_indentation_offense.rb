# nitrocop-config: EnforcedStyle: with_fixed_indentation

# First element on its own line, misaligned — RuboCop checks all elements
# including the first when it begins its own line
x = [
                    'An error occurred',
                    ^^^^^^^^^^^^^^^^^^^^ Layout/ArrayAlignment: Use one level of indentation for elements following the first line of a multi-line array.
                    'other string']
                    ^^^^^^^^^^^^^^^ Layout/ArrayAlignment: Use one level of indentation for elements following the first line of a multi-line array.

# Elements aligned with first element instead of fixed indentation
y = [1,
     2,
     ^^ Layout/ArrayAlignment: Use one level of indentation for elements following the first line of a multi-line array.
     3]
     ^^ Layout/ArrayAlignment: Use one level of indentation for elements following the first line of a multi-line array.
