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

# Block-pass receiver: RuboCop does not treat `&:strip` as a block for alignment
def ignored_organisations_string=(organisations_string)
  self.ignored_organisations = (organisations_string || "")
    .split(",")
    ^^^^^^ Layout/MultilineMethodCallIndentation: Align `.split` with `(organisations_string || "")` on line 65.
    .collect(&:strip)
    ^^^^^^^^ Layout/MultilineMethodCallIndentation: Align `.collect` with `(organisations_string || "")` on line 65.
      .compact
      ^^^^^^^^ Layout/MultilineMethodCallIndentation: Align `.compact` with `(organisations_string || "")` on line 65.
end

# []= receiver: square brackets are not parenthesized arg lists
def prepare_headers
  headers['Cookie'] = final_cookies_hash.
    map { |k, v| "#{Cookie.encode(k)}=#{Cookie.encode(v)}" }.join(';')
    ^^^ Layout/MultilineMethodCallIndentation: Align `map` with `final_cookies_hash` on line 73.
end

# Trailing-dot setter call: RuboCop checks setter methods like ordinary chains
trigger = proc do
  described_class.new(url: url, inputs: { name: 'value' }).
      nonce_name = 'stuff'
      ^^^^^^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 4) spaces for indentation of a chained method call.
end

# Repeated continuation dots should not inherit a bad column from the first one
def self.pull_request_filter
  where("contributions.user_id = aggregation_filters.user_id")
  .where("contributions.title ILIKE aggregation_filters.title_pattern")
  ^^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 0) spaces for indentation of a chained method call.
  .arel.exists.not
  ^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 0) spaces for indentation of a chained method call.
end

# RSpec stub chain: later dots still use the chain indent when the first continuation is wrong
before do
  allow(SteamCondenser::Community::SteamId).to receive(:steam_id_to_community_id)
                                              .with("STEAM_0:0:173804217")
                                              ^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 44) spaces for indentation of a chained method call.
                                              .and_return(76_561_198_307_874_162)
                                              ^^^^^^^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 44) spaces for indentation of a chained method call.
end

# A later continuation should not reuse an earlier column that was only valid
# because its receiver had a single-line block.
def household_size_options
  (0..10).map { |i| i }
         .unshift([t('common.prefer_not_to_answer'), -1])
         .unshift([nil, nil])
         ^^^^^^^^ Layout/MultilineMethodCallIndentation: Use 2 (not 7) spaces for indentation of a chained method call.
end
