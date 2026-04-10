my_method(1) \
^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  [:a]

foo && \
^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  bar

foo || \
^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  bar

my_method(1,
^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
          2,
          "x")

foo(' .x')
^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  .bar
  .baz

a =
^^^ Layout/RedundantLineBreak: Redundant line break detected.
  m(1 +
    2 +
    3)

b = m(4 +
^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
      5 +
      6)

raise ArgumentError,
^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
      "can't inherit configuration from the rubocop gem"

foo(x,
^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    y,
    z)
  .bar
  .baz

x = [
^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  1,
  2,
  3
]

y = {
^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  a: 1,
  b: 2
}

foo(
^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  bar(1, 2)
)

@count +=
^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  items.size

@@total +=
^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  items.size

$counter +=
^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  items.size

@cache ||=
^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  compute_value

@flag &&=
^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  check_flag

# Multiline %w array — RuboCop's safe_to_split? does not check arrays.
names = %w[
^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  alpha
  beta
  gamma
]

loop do
  if scan_progress_busy_duration > queue_timeout.to_i
    scan_progress_resp[:products].select { |p| p[:status] == 'B' }.each do |p|
      PWN::Plugins::BlackDuckBinaryAnalysis.abort_product_scan(
      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
        token: token,
        product_id: p[:product_id]
      )
    end
  end
end

scan_resp[:signals].each do |signal|
  cmd(
  ^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    gqrx_sock: gqrx_sock,
    cmd: "M #{mode_str} #{passband_hz}",
    resp_ok: 'RPRT 0'
  )
end

if dev_dependency_arr.include?(gem_name.to_sym)
  spec.add_development_dependency(
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    gem_name,
    gem_version
  )
else
  spec.add_dependency(
  ^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    gem_name,
    gem_version
  )
end

public_class_method def self.get_uris(opts = {})
  search_results = opts[:search_results]

  search_results.map do |search_results_hash|
    extract_uris(
    ^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
      search_results_hash: search_results_hash
    )
  end.flatten
rescue StandardError => e
  raise e
end

# String concatenation with backslash — the decoded values contain no \n,
# so safe_to_split? is true and these ARE offenses.
def internal_error
  Trip::InternalError.new(
  ^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    "The tracer encountered an internal error and crashed. " \
    "See #cause for details."
  )
end

def pause_error
  Trip::PauseError.new(
  ^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    "The pause_when Proc encountered an error and crashed. " \
    "See #cause for details."
  )
end

# Short string concatenation assignments — value has no \n, fits on one line.
msg = 'short string that ' \
^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
      'fits on one line'

error = "Node type must be any of #{types}, " \
^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
        "passed #{node_type}"

label = "#{name}::" \
^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
        "#{child_name}"

# Calls inside block bodies — individually checkable since the block
# boundary stops the walk-up in RuboCop's on_send.
existing_indexes_for(table_name).any? do |existing_index_column_names|
  leftmost_match?(
  ^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    haystack: existing_index_column_names,
    needle: indexed_column_names
  )
end

records.sort.each do |record|
  record.update(
  ^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    status: :processed,
    audit_comment: "bulk update"
  )
end

# Hash literal assignment — no unsafe constructs, fits on one line.
error = {
^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  key: "value",
  key2: "value2"
}

# Or-assignment with hash literal
configuration ||= {
^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  rbi: "output.rbi"
}

# Non-convertible block: call has args without parens, so RuboCop
# only checks the send portion (before do), not the whole block.
config.wrappers :default, class: :input,
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  hint_class: :field_with_hint do |b|
  b.use :placeholder
end

# Call nested in keyword hash argument of a chain — individually checked
# when the outer chain is too long to fit on one line.
expect(foo).to be_a(Parlour::RbsGenerator::Method) & have_attributes(
  name: 'foo',
  signatures: match_array([
              ^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
    have_attributes(
      parameters: [],
      return_type: nil,
    )
  ]),
)

# Assignment with method chain containing single-line block — not chained
# after the block, so Layout/SingleLineBlockChain does NOT take precedence.
# RuboCop flags these because block_node.parent is the assignment, not a send.
parameters = yard_parameters
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  .map { |x| process(x) }

parameters = split_type_parameters(type_parameters)
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  .map { |x| process(x) }

# Assignment with hash containing lambda (single-line block on `lambda` call)
options = {
^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  formatter: lambda { |tags| tags.join(" ") }
}.merge(options)

# Call with single-line block in argument — block parent is the outer call,
# not a chained send with dot, so Layout/SingleLineBlockChain doesn't apply.
scope = by_file_type(
^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  sources.map { |ext| lookup(ext) }
)

# Multi-write assignment with chain ending in single-line block
failures, successes = repository
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  .search_tests(input.split)
  .partition { |k, v| v.nil? }

validates :campaign, uniqueness: { scope: :user,
^ Layout/RedundantLineBreak: Redundant line break detected.
  message: "The same user may not join the same campaign twice" }

raise ArgumentError,
^ Layout/RedundantLineBreak: Redundant line break detected.
      "can't inherit configuration from the rubocop gem"

RUMOR_TYPES = {
^ Layout/RedundantLineBreak: Redundant line break detected.
  "RUMOR" => "this is rumor",
  "NOT_RUMOR" => "this is true",
}

it 'fails to resolve a dependency with an explicit source even if it can be ' \
^ Layout/RedundantLineBreak: Redundant line break detected.
   'resolved using the global sources' do
end

results.add_warning('summary', 'The summary should be a short ' \
^ Layout/RedundantLineBreak: Redundant line break detected.
  'version of `description` (max 140 characters).')

raise "[Xcodeproj] Type checking error: got `#{object.class}` " \
^ Layout/RedundantLineBreak: Redundant line break detected.
  "for attribute: #{inspect}" unless acceptable

raise "[Xcodeproj] unsupported key `#{key}` " \
^ Layout/RedundantLineBreak: Redundant line break detected.
  "(accepted `#{classes_by_key.keys}`) for attribute `#{inspect}`"

return if !checks.values.
          ^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  find { |c| c.check? page, [Element::Form::DOM, Element::Cookie::DOM], true }

# CJK characters: combined line is under 120 chars but over 120 bytes.
# RuboCop measures character length, not byte length.
DiceTable::Table.new(
^^^^^^^^^^^^^^^^^^^^ Layout/RedundantLineBreak: Redundant line break detected.
  "奇跡の触媒（エレメント）",
  "1D6",
  ["ワンド", "水晶玉", "カード", "ステッキ", "手鏡", "宝石"]
)

raise ArgumentError,
^ Layout/RedundantLineBreak: Redundant line break detected.
      "error message"

it 'fails to resolve a dependency with an explicit source even if it can be ' \
^ Layout/RedundantLineBreak: Redundant line break detected.
   'resolved using the global sources' do
end

results.add_warning('summary', 'The summary should be a short ' \
^ Layout/RedundantLineBreak: Redundant line break detected.
  'version of `description` (max 140 characters).')

raise "[Xcodeproj] Type checking error: got `#{object.class}` " \
^ Layout/RedundantLineBreak: Redundant line break detected.
  "for attribute: #{inspect}" unless acceptable

raise "[Xcodeproj] unsupported key `#{key}` " \
^ Layout/RedundantLineBreak: Redundant line break detected.
  "(accepted `#{classes_by_key.keys}`) for attribute `#{inspect}`"

!current_course_user&.
 ^ Layout/RedundantLineBreak: Redundant line break detected.
  email_unsubscribed

), public: File.read(
           ^ Layout/RedundantLineBreak: Redundant line break detected.
  path

Datadog::Tracing::Contrib::Sidekiq::Patcher
^ Layout/RedundantLineBreak: Redundant line break detected.
  .instance_variable_get(:@patch_only_once)

!current_course_user&.
 ^ Layout/RedundantLineBreak: Redundant line break detected.
  email_unsubscribed

Datadog::Tracing::Contrib::Sidekiq::Patcher
^ Layout/RedundantLineBreak: Redundant line break detected.
  .instance_variable_get(:@patch_only_once)

Datadog::Tracing::Contrib::Sidekiq::Patcher
^ Layout/RedundantLineBreak: Redundant line break detected.
  .instance_variable_get(:@patch_only_once)

o.col_type.nil? \
^ Layout/RedundantLineBreak: Redundant line break detected.
  foo

expect(ForestLiana::DecorationHelper)
^ Layout/RedundantLineBreak: Redundant line break detected.
  .to receive(:decorate_for_search)
