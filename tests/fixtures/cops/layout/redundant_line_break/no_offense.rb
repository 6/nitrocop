my_method(1, 2, "x")

foo(a, b)

a = if x
      1
    else
      2
    end

foo \
  && bar

foo \
  || bar

x = 42

# Backslash in a comment line should not trigger
# 'foo' \
#   'bar'

# This is a YARD example with backslash \
# continuation that is just a comment

# A line that would be too long when combined (exceeds 120 chars):
this_is_a_very_long_method_name_that_makes_the_line_quite_long(argument_one, argument_two, argument_three) \
  .and_then_another_long_chain_call

MSG = 'This is a long error message string that definitely ' \
      'exceeds one hundred and twenty characters when concatenated together'

# String concatenation where the value contains \n — safe_to_split? is false
expect(output)
  .to eq('[modify] A configuration is added into ' \
         "#{path}.\n")

# Method call on a single line is fine
my_method(1, 2, "x")

# Multiline method call that would exceed 120 chars when joined on one line
my_method(1111111111111111,
          2222222222222222,
          3333333333333333,
          4444444444444444,
          5555555555555555,
          6666666666666666,
          7777777777777777)

# Method call with comments on intermediate lines
my_method(1,
          2,
          "x") # X

# Assignment containing an if expression
a =
  if x
    1
  else
    2
  end

# Assignment containing a case expression
a =
  case x
  when :a
    1
  else
    2
  end

# Method call with a do block (InspectBlocks: false by default)
a do
  x
  y
end

# Assignment containing a begin-end expression
a ||= begin
  x
  y
end

# Complex method chain that is too long for a single line
node.each_node(:dstr).select(&:heredoc?).map { |n| n.loc.heredoc_body }.flat_map { |b| (b.line...b.last_line).to_a }

# Method call with heredoc argument
foo(<<~EOS)
  xyz
EOS

# Method call with a multiline string argument
foo('
  xyz
')

# Multiline regexp assignment — RuboCop's safe_to_split? sees regexp body
# strings in Parser AST, so this is NOT an offense.
GROUPED_INPUT_PATTERN = /
  \A
  (?<key>.+)
  \((?<index>\d+)i\)
  \z
/x.freeze

# Quoted symbol with a single newline
foo(:"
")

# Binary expression containing an if expression
a +
  if x
    1
  else
    2
  end

# Modified singleton method definition
x def self.y
    z
  end

# Multiline block without a chained method call (InspectBlocks: false)
f do
end

# Method call chained onto a multiline do block (InspectBlocks: false)
e.select do |i|
  i.cond?
end.join

# A method call chained onto a single line block (Layout/SingleLineBlockChain precedence)
e.select { |i| i.cond? }
 .join

# Stabby lambda argument on a dotted call — Layout/SingleLineBlockChain takes precedence.
assoc.has_many :connections, -> { order 'connections.position ASC' },
               inverse_of: :affiliate

# Outer call containing a dotted call with a stabby lambda argument — RuboCop
# still defers because the descendant lambda's containing send has a dot.
assert_equal(
  '<div class="blue and green"></div>',
  Papercraft.html(-> { div class: 'blue and green' })
)

# Backslash continuation where the expression continues across comma-terminated
# lines. RuboCop measures the full attr_reader call, not just the first symbol.
class PhaseTwoLongAttrReader
  attr_reader \
    :health_metrics,
    :settings,
    :agent_settings,
    :logger,
    :remote,
    :profiler,
    :runtime_metrics,
    :telemetry,
    :crashtracker
end

# Index access call chained — see RuboCop's index_access_call_chained? check
# hash[:foo] \
#   [:bar]

# Multiline method chain where full chain exceeds 120 chars — inner calls must not be flagged
keys =
  ApiKey
    .where(hidden: false, archived: false, organization_id: current_organization.id)
    .includes(:user, :permissions, :audit_logs)
    .includes(:created_by)

# Method chain where the outermost is too long, inner nodes should not be individually checked
logs
  .includes(:user, :actor, post: [:topic, :category])
  .references(:user, :actor)
  .where("created_at > ? AND action_type IN (?)", 30.days.ago, UserAction.types[:posted])
  .order(created_at: :desc)

# Constant receiver with long chain — outermost too long, inner nodes must be skipped
Theme
  .not_components
  .where("themes.id = ? OR themes.enabled = ?", SiteSetting.default_theme_id, true)
  .includes(:theme_site_settings)

# Assignment with a multiline chain on the RHS that exceeds 120 chars
result = Record
  .where(status: :active, role: "admin", organization_id: current_organization.id)
  .includes(:organization, :permissions, :audit_trail)
  .order(created_at: :desc)
  .limit(100)

# Chain where an inner call spans only 2 lines but full chain is long
User
  .active
  .where(role: "manager", department_id: Department.find_by(name: "Engineering").id)
  .includes(:department, :reports, :direct_reports, :manager)
  .order(:last_name, :first_name)

# Assignment with a block on RHS (InspectBlocks: false should skip these)
wrap = lambda do |_, inner|
  inner.call
end

# Instance variable assignment with a block on RHS
@thread = Thread.new do
  listen
end

# Assignment with a method call that has a multiline do block
result = items.select do |item|
  item.active?
end

# Direct multiline call argument: RuboCop walks up from `.where` to the
# outer `SeedDump.dump(...)` send, so the inner call is not checked alone.
SeedDump.dump(EventInstance
              .where('created_at >= :start_date', { start_date: 1.month.ago }),
              file: 'db/seeds/event_instances.rb')

# Assignment with a multiline brace block
handler = proc { |x|
  process(x)
}

# Multiline `or` keyword without backslash — RuboCop checks operator_keyword?
# and only flags if line ends with backslash; without backslash, not an offense
x = foo or
  bar

# Multiline `and` keyword without backslash — same as above
x = foo and
  bar

# Method chain with multiline brace block (InspectBlocks: false)
# RuboCop walks up from `join` send, but `map { ... }` has a multiline block
# descendant, so configured_to_not_be_inspected? returns true
items.map { |i|
  i.name
}.join(', ')

# Backslash continuation with a multiline do block (InspectBlocks: false)
# The do block is multiline, so the expression is not inspected
items.each do |item|
  process(item)
end \
  .tap { |r| log(r) }

# Method call with \r\n escape — decoded value contains \n, so safe_to_split?
# returns false and RuboCop does not flag it.
PWN::Plugins::Serial.request(
  serial_obj: serial_obj,
  payload: "AT+CLAC\r\n"
)

# Method call with interpolated string containing \r\n escape
PWN::Plugins::Serial.request(
  serial_obj: serial_obj,
  payload: "ATDT#{voicemail_num};\r\n"
)

# Multiline parenthesized group — outer call has a multiline ParenthesesNode
# descendant so safe_to_split? is false. The inner expression is too long to
# fit on one line, so it's also not flagged.
foo_method_with_long_name(
  (variable_one_long_name + variable_two_long_name + variable_three_long_name +
   variable_four_long_name + variable_five_long_name + variable_six_long_name + variable_seven_long_name)
)

# Assignment with multiline %q{} string inside a method body
# The %q{} string contains newlines so safe_to_split? should return false.
# Previously missed because UnsafeRangeCollector did not recurse into DefNode.
def test_it
  source = %q{
p id="test"
}

  assert_html '<p>x</p>', source
end

# Assignment with multiline %Q{} string inside a class/method
class TestClass
  def test_method
    template = %Q{
<div>#{name}</div>
}
    render template
  end
end

# Assignment with if on RHS inside a nested class/method
class Config
  def resolve
    @prefix = if @prefix
                "#{@prefix}[#{name}]"
              else
                name
              end
  end

  def lookup
    value =
      if key.present?
        store[key]
      else
        default
      end
    value
  end

  def status_code
    @code =
      if code.is_a?(Symbol)
        begin
          lookup(code)
        rescue ArgumentError
          nil
        end
      else
        code
      end
  end
end

# Assignment with case on RHS inside a method
def kind
  result = case input
           when :a then 1
           when :b then 2
           else 0
           end
  result
end

# Multiline call inside `||` without backslash — RuboCop walks up from
# the inner send through the BinaryOperatorNode (OrNode). operator_keyword?
# returns true, but the operator line does NOT end with `\`, so
# require_backslash? returns false and there is no offense.
foo || bar(
  1, 2
)

# Same with `&&`
foo && bar(
  1, 2
)

# Assignment with || — the assignment is flagged separately if it fits on
# one line, but the inner call `bar(1, 2)` should NOT be flagged because
# it is inside the OrNode without backslash.
destroy || raise(
  ActiveRecord::RecordNotDestroyed.new("Failed to destroy the record", self)
)

# Chain with backslash content in regex — combined line exceeds 120 chars
# when backslashes are preserved. The \1_\2 and \d are regex content,
# not line continuations.
module FPTest
  class Converter
    def self.underscore(str)
      str.gsub(/::/, '/').
        gsub(/([A-Z]+)([A-Z][a-z])/,'\1_\2').
        gsub(/([a-z\d])([A-Z])/,'\1_\2').
        tr("-", "_").
        downcase
    end
  end
end

# Backslash continuation followed by case expression — the case extends
# beyond the backslash group and can't be collapsed to one line.
@parlour = options[:parlour] || \
  case @mode
  when :rbi
    "rbi"
  when :rbs
    "rbs"
  end

# Backslash continuation followed by modifier `until`
current_context = current_context.parent \
  until current_context.is_a?(NamespaceObject)

# Condition header with only the keyword before the backslash. RuboCop does not
# flag these; it only flags when condition code appears before the backslash.
if \
  foo && bar
then
  baz
end

if \
  left.type == :send and left.children.length == 3 and
  left.children[1] == :+
then
  left = collapse_strings(left)
end

# Ruby's $\ global variable is not a line-continuation marker.
alias $ORS $\
alias $OUTPUT_RECORD_SEPARATOR $\
@origOutputSep = $\

# Backslash continuation with an inline comment in the continued argument list.
# RuboCop will not collapse the comment onto a single line.
class AttrReaderWithComment
  attr_reader \
    :filepath, # DEV(1.0): Rename to `uds_path`
    :timeout
end

# Backslash string continuation inside an `if` expression branch. The enclosing
# conditional makes the whole assignment unsafe to collapse.
if raise_error
  error_msg = if nilable
    "The setting foo expects a " \
    "String or nil, but value was bad." \
  else
    "The setting foo expects a " \
    "String, but value was bad." \
  end

  error_msg = "#{error_msg} Please update your configure block. "
end

# Multiline interpolated strings are unsafe to collapse even when the
# interpolated call would fit on one line by itself.
html = "<h2>#{
  ERB::Escape.html_escape((title).to_s)
}</h2>"

# Class inheritance with backslash continuation is not an inspectable
# single-line expression for this cop.
class Course::Assessment::Submission::LogsController < \
  Course::Assessment::Submission::Controller

  def index
    authorize!(:manage, @assessment)
  end
end

# Backslash groups inside a larger multiline operator chain should not be
# reported independently; RuboCop judges the full expression.
Arel.quoted('"') \
+ expr
    .coalesce('')
    .replace('\\', '\\\\')
    .replace('"', '\"')
    .replace("\b", '\b')
    .replace("\f", '\f')
    .replace("\n", '\n')
    .replace("\r", '\r')
    .replace("\t", '\t') \
+ '"'

# Backslash predicates inside multiline ternaries are accepted by RuboCop even
# when the predicate itself would fit on one line.
comments.any? \
  ? comments.map { |c| options.indented(indent_level, "# #{c}") }
  : []

certs_dir && !certs_dir.empty? \
  ? 'codekitchen/dinghy-http-proxy:2.5.10' \
  : 'freedomben/dory-http-proxy:2.6.2.2'

# Hash key/value continuation inside a larger hash. RuboCop judges the hash as
# a whole and does not report the continued key/value line independently.
form_fields = {
  "#{PAGE1_KEY}.VeteransServiceNumber_If_Applicable[0]": \
  data.veteran_service_number,
  "#{PAGE1_KEY}.SomeExtremelyLongFieldNameThatKeepsTheWholeHashFromFittingOnOneLine[0]": data.other
}

# Class expression receiver with a multiline body. RuboCop does not collapse
# the class body just to attach the trailing `.new`.
let!(:test_object) do
  class TestController < ApplicationController
    include TestObjectContent

    self
  end.new
end

# Inside an `or` / `and` keyword expression, RuboCop walks up to the operator
# node before checking suitability. If the joined outer expression would
# exceed `MaxLineLength`, the offense is suppressed even when the inner
# `raise`/call by itself would fit on a single line.
def configure(my_proc, req_arity, var)
  case my_proc
  when Proc
    arity = my_proc.arity
    (arity == req_arity) or \
      raise ArgumentError,
            "#{var}=#{my_proc.inspect} has invalid arity: " \
            "#{arity} (need #{req_arity})"
  end
end

# `pipe arg1, arg2` style call where the joined call exceeds MaxLineLength.
# Phase 2's combined-line check undercounted by stripping a trailing `\\` from
# the line one past the backslash sequence (which holds the closing arg / call,
# not a continuation marker), and `covered_by_checked_chain` failed because the
# AST chain starts after the indent while the group started at column 0.
module Builder
  class Remote
    def inspect_remote_context
      pipe \
        docker(:context, :inspect, remote_context_name, "--format", ENDPOINT_DOCKER_HOST_INSPECT),
        grep("-xq", remote)
    end
  end
end

# Backslash continuation between method-chain segments survives RuboCop's
# `to_single_line` chain-dot collapse regex `/\n\s*(?=(&)?\.\w)/`, which strips
# the newline+indent but leaves the trailing `\` in place. The joined length
# therefore exceeds 120 chars and `too_long?` suppresses the offense.
on_supported_os.each do |os, facts|
  describe 'datadog::ubuntu' do
    let(:facts) { facts }
    context 'with debian' do
      context 'with reports' do
        context 'when ruby-devel installed' do
          context 'with provider option' do
            it do
              is_expected.to contain_package('ruby-devel')\
                .with_ensure('installed')\
                .that_comes_before('Package[dogapi]')
            end
          end
        end
      end
    end
  end
end
