# nitrocop-config: EnforcedStyle: always_braces

# Block nested inside ignored lambda arg body through a hash literal
register_placeholder :path, ->(resource) do
  {
    raw_value: resource.relative_path_basename_without_prefix.tap do |path|
      path
    end,
  }
end

# Block nested inside ignored lambda arg body through a heredoc interpolation
render html: -> {
  <<~HTML
    <ul>
    #{html_map 3.times do |i| <<~INNER
      <li>#{i}</li>
    INNER
    end}
    </ul>
  HTML
}
