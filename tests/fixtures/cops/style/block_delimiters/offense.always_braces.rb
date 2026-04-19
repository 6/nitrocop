# nitrocop-config: EnforcedStyle: always_braces

xml.wrapper do
            ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  xml << yield
end

# encoding: iso-8859-1
context "Rabl::Engine" do
                       ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  helper(:rabl) { "café" }
end
