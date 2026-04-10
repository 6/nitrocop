# nitrocop-config: EnforcedStyle: forbid_logical_operators

# Pattern-matching guards are not regular `unless` statements for this cop.
case url_parts
in _, "pinterest", _, username, *rest unless username.in?(RESERVED_NAMES) || subdomain == "api"
  @username = username
end
