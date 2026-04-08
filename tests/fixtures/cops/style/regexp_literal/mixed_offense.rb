# nitrocop-config: EnforcedStyle: mixed
url = params[:repo].gsub(/\/$/, '')
                         ^^^^^ Style/RegexpLiteral: Use `%r` around regular expression.
