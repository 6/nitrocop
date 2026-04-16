# nitrocop-config: EnforcedStyle: indented_relative_to_receiver
# nitrocop-expect: 3:1 Layout/MultilineMethodCallIndentation: Indent `.b` 2 spaces more than `a` on line 2.
# nitrocop-expect: 7:0 Layout/MultilineMethodCallIndentation: Indent `b` 2 spaces more than `a` on line 6.
# nitrocop-expect: 11:3 Layout/MultilineMethodCallIndentation: Indent `b` 2 spaces more than `a` on line 10.
# nitrocop-expect: 16:4 Layout/MultilineMethodCallIndentation: Indent `c` 2 spaces more than `a` on line 14.
# nitrocop-expect: 21:1 Layout/MultilineMethodCallIndentation: Indent `b` 2 spaces more than `a` on line 20.
# Basic: 1 space indent instead of 2
a
 .b

# Trailing dot: no indent
a.
b

# Trailing dot: 3 spaces instead of 2
a.
   b

# 3rd line extra indentation (trailing dot)
a.
  b.
    c

# Array item trailing dot
[
 a.
 b
]

# Hash pair value: shifted left
# nitrocop-expect: 28:1 Layout/MultilineMethodCallIndentation: Indent `.veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name` 2 spaces more than `VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName` on line 27.
def foo
  bar(
    key: VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName
 .veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name
  )
end

# Hash pair value: shifted right
# nitrocop-expect: 36:16 Layout/MultilineMethodCallIndentation: Indent `.veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name` 2 spaces more than `VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName` on line 35.
def foo
  bar(
    key: VeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeryLongClassName
                .veeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeery_long_method_name
  )
end

# Unary operator receiver
# nitrocop-expect: 43:2 Layout/MultilineMethodCallIndentation: Indent `.nil?` 2 spaces more than `0` on line 42.
def foo
  !0
  .nil?
end

# Hash literal receiver: misaligned (dot of .keys is at col 14, expect col 16)
# nitrocop-expect: 48:14 Layout/MultilineMethodCallIndentation: Indent `.first` 2 spaces more than `.keys` on line 47.
{ a: 1, b: 2 }.keys
              .first

# Proc call without selector
# nitrocop-expect: 52:1 Layout/MultilineMethodCallIndentation: Indent `.(` 2 spaces more than `a` on line 51.
a
 .(args)
