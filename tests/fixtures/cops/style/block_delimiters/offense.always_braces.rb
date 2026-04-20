# nitrocop-config: EnforcedStyle: always_braces

front_matter do
             ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  layout :default
  custom_var 123
end

xml.wrapper do
            ^^ Style/BlockDelimiters: Prefer `{...}` over `do...end` for blocks.
  xml << yield
end
