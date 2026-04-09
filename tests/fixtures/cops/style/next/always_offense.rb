# nitrocop-config: EnforcedStyle: always

items.each do |from|
  raise(
  ^^^^^ Style/Next: Use `next` to skip iteration.
    MyError,
    :file => from
  ) unless File.exist?(from)
end

items.each do |item|
  work do
  ^^^^^^^ Style/Next: Use `next` to skip iteration.
    step_one
    step_two
  end if condition
end

# parenthesized modifier if as sole block body
AppConfig.providers.fetch('identity', []).map do |provider|
  ({ name: provider, href: send("#{provider}_oauth_path") } if ENV["#{provider.upcase}_APP_KEY"])
   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/Next: Use `next` to skip iteration.
end.compact
