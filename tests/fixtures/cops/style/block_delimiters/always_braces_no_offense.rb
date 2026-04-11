# nitrocop-config: EnforcedStyle: always_braces
register_placeholder :path, ->(resource) do
  {
    raw_value: resource.relative_path_basename_without_prefix.tap do |path|
      path
    end,
  }
end
