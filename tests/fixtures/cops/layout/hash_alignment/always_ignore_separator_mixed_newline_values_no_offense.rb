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
