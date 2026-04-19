# nitrocop-config: EnforcedStyle: indented
# Matcher chains nested inside a non-parenthesized argument keep the outer
# visual indentation instead of indenting relative to the inner matcher.
expect { service.call }.to \
  change { visit.reload.name }
  .from("a")
  .to("b")
