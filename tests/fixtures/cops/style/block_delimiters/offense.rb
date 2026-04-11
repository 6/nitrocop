items.each do |x| puts x end
           ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for single-line blocks.

items.map {
          ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
  |x| x * 2
}

[1, 2].each do |i| i + 1 end
            ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for single-line blocks.

items.map {
          ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
  items.select {
    true
  }
}

# Chained blocks: only the outermost (last in chain) is flagged
items.select {
  x.valid?
}.reject {
  x.empty?
}.each {
       ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
  puts x
}

# super with args and multi-line braces should be flagged
super(arg) {
           ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
  do_something
}

# forwarding super (no args) with multi-line braces
super {
      ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
  yield if block_given?
  process
}

expect {
       ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
  subject.log("a")
  subject.debug("b")
}.to output("a\nb\n").to_stdout

# Explicit begin/rescue still treats the block as procedural
begin
  Timeout.timeout(20) {
                      ^ Style/BlockDelimiters: Prefer `do...end` over `{...}` for multi-line blocks.
    sleep 0.1 while !server.ready?
  }
rescue Exception => e
  raised = true
end
