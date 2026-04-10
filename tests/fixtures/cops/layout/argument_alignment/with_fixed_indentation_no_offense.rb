# nitrocop-config: EnforcedStyle: with_fixed_indentation

# Correctly indented at call_indent + 2
foo(1,
  2,
  3)
bar(:a,
  :b,
  :c)

# Sole keyword hash at correct indentation
Report.new(
  sitemap: 200,
  issues: 'sort'
)

# Sole braced hash at fixed indentation
expect(subject).to eq(
  {
    a: 1,
    b: 2,
  }
)
