# nitrocop-config: EnforcedStyle: separator, separator, always_ignore
hash = {
  "xml"      =>
      proc { xml },
  "uiinput"  => false,
  "uidom"    =>
      proc { ui_inputs },
  "body"     => !body.empty?
}

resource = {
  cover_url:
    if attached?
      url
    end,
  title: name,
  body: excerpt
}

cases = {
  "Empty annotation name":
    ResourceRestriction.new(name: "", value: "val"),
  "Double slash":
    ResourceRestriction.new(name: "a//b", value: "val"),
  "Nested Array":
    ResourceRestriction.new(name: "a[2]/c", value: "val")
}

rocket_cases = {
  "Empty annotation name" =>
    ResourceRestriction.new(name: "", value: "val"),
  "Double slash" =>
    ResourceRestriction.new(name: "a//b", value: "val"),
  "Nested Array" =>
    ResourceRestriction.new(name: "a[2]/c", value: "val")
}
