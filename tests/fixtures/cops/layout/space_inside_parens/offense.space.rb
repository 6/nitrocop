# nitrocop-config: EnforcedStyle: space
case p
in Point(*, 1, *a)
         ^ Layout/SpaceInsideParens: No space inside parentheses detected.
                 ^ Layout/SpaceInsideParens: No space inside parentheses detected.
  a
end

# FN fix: binstub-style `abort("...<real-newline>...")` — Parser splits the
# string into separate tSTRING_BEG/tSTRING_END tokens, so RuboCop fires the
# close-side check on the `)` line.
abort("hello
      ^ Layout/SpaceInsideParens: No space inside parentheses detected.
world")
      ^ Layout/SpaceInsideParens: No space inside parentheses detected.

# FN fix: same pattern with single-quoted multi-line string. Parser always
# splits single-quoted strings on real newlines.
invoke('line1
       ^ Layout/SpaceInsideParens: No space inside parentheses detected.
line2')
      ^ Layout/SpaceInsideParens: No space inside parentheses detected.
