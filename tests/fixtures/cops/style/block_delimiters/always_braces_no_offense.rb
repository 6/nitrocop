# nitrocop-config: EnforcedStyle: always_braces

# Blocks nested inside a lambda passed to a non-parenthesized call are ignored.
register_placeholder :path, ->(resource) do
  {
    raw_value: resource.relative_path_basename_without_prefix.tap do |path|
      path.delete_prefix! "content/"
    end,
  }
end

# The same ignored-body traversal applies through interpolated strings.
render html: -> {
  <<~HTML
    <ul>
    #{html_map 3.times do |i|
      <<~HTML
        <li>#{text -> { i }}</li>
      HTML
    end}
    </ul>
  HTML
}
