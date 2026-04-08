# nitrocop-config: EnforcedStyle: with_fixed_indentation

# with_fixed_indentation: first argument on continuation line is also checked
foo(1,
    2,
    ^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.
    3)
    ^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.
bar(:a,
      :b,
      ^^^^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.
      :c)
      ^^^^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.

# Sole keyword hash: all pairs on continuation lines are checked
Report.new(
    sitemap: 200,
    ^^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.
    issues: 'sort'
    ^^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.
)

# Sole braced hash with multiple pairs is still checked
expect(subject).to eq(
    {
    ^ Layout/ArgumentAlignment: Use one level of indentation for arguments following the first line of a multi-line method call.
      a: 1,
      b: 2,
    }
)
