# nitrocop-config: EnforcedStyle: semantic
(items.map do |p|
           ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for functional blocks.
  p
end).compact

begin
  Timeout.timeout(20) {
                      ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for procedural blocks.
    sleep 0.1 while !server.ready?
  }
rescue Exception => e
  raised = true
end
