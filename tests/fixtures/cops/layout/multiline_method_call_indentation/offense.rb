foo
  .bar
    .baz
    ^^^ Layout/MultilineMethodCallIndentation: Align `.baz` with `foo` on line 1.

thing
  .first
  .second
      .third
      ^^^ Layout/MultilineMethodCallIndentation: Align `.third` with `thing` on line 5.

query
  .select('foo')
  .where(x: 1)
    .order(:name)
    ^^^ Layout/MultilineMethodCallIndentation: Align `.order` with `query` on line 10.

# Block chain continuation: .sort_by should align with .with_index dot
frequencies.map.with_index { |f, i| [f / total, hex[i]] }
           .sort_by { |r| -r[0] }
           ^^^ Layout/MultilineMethodCallIndentation: Align `.sort_by` with `.with_index` on line 16.

# Multiline receiver chain with single-line block: .sort_by should align with .with_index dot
submission.template_submitters
          .group_by.with_index { |s, index| s['order'] || index }
          .sort_by(&:first).pluck(1)
          ^^^ Layout/MultilineMethodCallIndentation: Align `.sort_by` with `.with_index` on line 21.

# Hash pair value: chain should align with chain root start column
foo(key: receiver.chained
                          .misaligned)
                          ^^^ Layout/MultilineMethodCallIndentation: Align `.misaligned` with `receiver.chained` on line 25.

bar = Foo
  .a
  ^^ Layout/MultilineMethodCallIndentation: Align `.a` with `Foo` on line 28.
      .b(c)

# Trailing dot: unaligned methods (aligned style)
User.a
  .b
  ^^ Layout/MultilineMethodCallIndentation: Align `.b` with `.a` on line 33.
 .c
 ^^ Layout/MultilineMethodCallIndentation: Align `.c` with `.a` on line 33.

# Trailing dot: misaligned in assignment
a = b.c.
 d
 ^ Layout/MultilineMethodCallIndentation: Align `d` with `b` on line 38.

# Unaligned method in block body
a do
  b.c
    .d
    ^^ Layout/MultilineMethodCallIndentation: Align `.d` with `.c` on line 43.
end

# Hash pair value: misaligned multi-dot chain
method(key: value.foo.bar
                    .baz)
                    ^^^^ Layout/MultilineMethodCallIndentation: Align `.baz` with `value.foo.bar` on line 48.

# Aligned style fallback: implicit receiver chain with no indent
where("first_condition")
.where("second_condition")
^ Layout/MultilineMethodCallIndentation: Use 2 (not 0) spaces for indentation of a chained method call.

# Block continuation: .map has block, aligned with receiver's continuation dot
# RuboCop accepts .map because find_continuation_node returns .select's dot
def foo
  MyClass.all
    .select("name")
    ^^ Layout/MultilineMethodCallIndentation: Align `.select` with `.all` on line 58.
    .map { |e| e.name }
end

.compact
^ Layout/MultilineMethodCallIndentation: Align `.compact` with `(organisations_string || "")` on line 238.

.create_for_missing_param(@method_description, name)
^ Layout/MultilineMethodCallIndentation: Align `.create_for_missing_param` with `Apipie::Generator::Swagger::ParamDescription` on line 82.

map{ |rule| rule.split( hash_delimiter, 2 ) }]
^ Layout/MultilineMethodCallIndentation: Align `map` with `string_or_hash.to_s.split( /[\n\r]/ ).reject( &:empty? ).` on line 367.

map { |var| var % payload } | [payload]
^ Layout/MultilineMethodCallIndentation: Align `map` with `[ ';%s', "\";%s#", "';%s#" ].` on line 53.

gsub( '__TIME__', payload_delay_from_options( options ) )
^ Layout/MultilineMethodCallIndentation: Align `gsub` with `injected.` on line 376.

map { |k, v| "#{Cookie.encode( k )}=#{Cookie.encode( v )}" }.join( ';' )
^ Layout/MultilineMethodCallIndentation: Align `map` with `final_cookies_hash.` on line 816.
