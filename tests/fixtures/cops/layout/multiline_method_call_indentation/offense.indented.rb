# nitrocop-config: EnforcedStyle: indented
# The plain matcher chain still uses normal 2-space indentation.
change { visit.reload.name }
.from("a")
^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 0) spaces for indentation of a chained method call.
.to("b")
^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 0) spaces for indentation of a chained method call.

# Nested matcher chains do not inherit the outer expectation indentation.
expect { described_class.perform_now }.to \
  have_enqueued_job(Job)
    .with(1)
    ^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 4) spaces for indentation of a chained method call.
    .and have_enqueued_job(Job)
    ^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 4) spaces for indentation of a chained method call.
    .with(2)
    ^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 4) spaces for indentation of a chained method call.
