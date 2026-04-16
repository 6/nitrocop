# nitrocop-config: EnforcedStyle: indented
@payloads[platform] = [ ';%s', "\";%s#", "';%s#" ].
    map { |var| var % payload } | [payload]
    ^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 4) spaces for indentation of a chained method call.
