# nitrocop-config: EnforcedStyle: semantic

def matches?(value)
  klasses.any? { |klass| value.is_a?(klass) }
               ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
rescue NoMethodError
  false
end

(queries.map do |query|
             ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for functional blocks.
  query.strip
end).join(",")

master.job_queue << -> do
  Steep.logger.info { "Type checking for stats..." }
                    ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
  progress = master.work_done_progress(typecheck_guid)
  master.start_type_check(last_request: nil, progress: progress, include_unchanged: true, report_progress_threshold: 0, needs_response: true)
end

def examples
  example { "a b" }
          ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
  example { "ab" }
end

def view_template
  a(href: " javascript:alert(1)") { "XSS" }
                                  ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
  a(href: "javascript :alert(1)") { "XSS" }
end

outer do
  example { expect(call(<<~CODE)).to eq("1\n") }
          ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
  1

  # ~> 2
  CODE

  example { expect(call(<<~CODE)).to eq("1\n") }
  1 # ~> error

  # ~> error again
  CODE
end

outer do
  example { expect(call(<<-CODE)).to eq "1\n" }
          ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
  1

    # ~> 2
  CODE

  example { expect(call(<<-CODE)).to eq "1\n" }
  1 # ~> error

    # ~> error again
  CODE
end
