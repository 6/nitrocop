# nitrocop-config: EnforcedStyle: always_braces

# Block inside lambda body returned as a positional argument is suppressed
register_placeholder :path, ->(resource) do
  {
    raw_value: resource.relative_path_basename_without_prefix.tap do |path|
      if resource.site.config["collections_dir"].length.positive?
        path.delete_prefix! "#{resource.site.config["collections_dir"]}/"
      end

      Bridgetown::Utils.chomp_locale_suffix!(path, resource.data.locale)
    end,
  }
end

# Block inside heredoc interpolation within an ignored lambda body is suppressed
render html->{ <<~HTML
  <ul>
  #{html_map 3.times do |i| <<~HTML
    <li>#{text->{ i }}</li>
  HTML
  end}
  </ul>
HTML
}
