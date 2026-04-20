# nitrocop-config: EnforcedStyle: always_braces
# encoding: iso-8859-1

front_matter do
             ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  layout :default
  custom_var 123
end

xml.wrapper do
            ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  xml << yield
end

context "Rabl::Engine" do
                       ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  value = "\x81\xA4user\x80"
end
