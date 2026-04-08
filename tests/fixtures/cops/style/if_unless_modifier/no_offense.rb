do_something if x

do_something unless x

# Another statement on the same line — RuboCop skips long modifier forms here
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa if condition; bar

if x
  do_something
else
  do_other
end

if x
  do_something
  do_other
end

unless x
  foo
  bar
end

if x
  very_long_method_name_that_would_exceed_the_max_line_length_when_used_as_a_modifier_form_together_with_the_condition
end

# elsif branches should not be flagged
if x
  do_something
elsif y
  do_other
end

if a
  one
elsif b
  two
elsif c
  three
end

# Multi-line body: can't be converted to modifier form
if condition
  method_call do
    something
  end
end

unless condition
  class Foo
    bar
  end
end

# Body with EOL comment should not suggest modifier
unless a
  b # A comment
end

# Long comment on condition line would make the modifier form too long
unless @values[item.key] # this is important to not verify if there already is a value there
  item.verify_block.call(item.default_value)
end

# Comment before end should not suggest modifier
if a
  b
  # comment
end

# defined? in condition should not suggest modifier — semantics change in modifier form
if defined?(RubyVM::YJIT.enable)
  RubyVM::YJIT.enable
end

unless defined?(some_variable)
  some_variable = 'default'
end

# Local variable assignment in condition should not suggest modifier
if (x = something)
  use(x)
end

# Assignment embedded in condition (non_eligible_condition)
if x = compute_value
  process(x)
end

# Local variable ||= in condition should not suggest modifier
if (severity ||= UNKNOWN) > (@max_severity ||= severity)
  @max_severity = severity
end

# Local variable operator assignment in condition should not suggest modifier
if (iterations += 1) > MAX_ITERATIONS
  raise InfiniteCorrectionLoop.new(source.path, offenses_by_iteration)
end

# Local multiple assignment in condition should not suggest modifier
unless (prev_arg_node, default_value_node = default_value_argument_and_block(node.parent))
  return
end

if (sort_node, sorter, accessor = redundant_sort?(node))
  return [node, sort_node, sorter, accessor]
end

# Nested conditional in body: don't suggest modifier for outer if
if x
  return true if y
end

unless a
  do_something unless b
end

# Ternary in body (Prism parses ternary as IfNode)
if condition
  x ? do_this : do_that
end

# Ternary nested inside assignment in body (nested_conditional)
if archived.in?([true, false])
  @template.archived_at = archived == true ? Time.current : nil
end

# Ternary nested inside assignment in body (nested_conditional)
if has_and_mask && !and_mask_bits_for_row.empty?
  a = and_mask_bits_for_row[x_pixel] == 1 ? 0 : 255
end

# Multi-line assignment: modifier form would need parens and exceed line length
class Foo
  def load_resubmit_submitter
    @resubmit_submitter =
      if params[:resubmit].present? && !params[:resubmit].in?([true, 'true'])
        Submitter.find_by(slug: params[:resubmit])
      end
  end
end

# Chained call after end — RuboCop skips chained if-end (node.chained?)
if test
  something
end.inspect

if test
  something
end&.inspect

# Binary operator after end — not convertible (code_after)
if test
  1
end + 2

# Comment on end line — RuboCop: line_with_comment?(node.loc.last_line)
if a
  b
end # comment

# Named regexp capture in condition — modifier form changes semantics
if /(?<name>\d+)/ =~ input
  name
end

# Bare regexp literal on the LHS of =~ should not suggest modifier form
if /foo/ =~ bar
  baz
end

# Endless method definition in body — Style/AmbiguousEndlessMethodDefinition conflict
if condition
  def method_name = body
end

if condition
  def self.method_name = body
end

# Pattern matching (in) in condition — modifier form changes variable scoping
if [42] in [x]
  x
end

# Multiline condition (nonempty_line_count > 3) — RuboCop won't suggest modifier
if a &&
   b
  do_something
end

unless some_long_condition ||
       another_condition
  do_something
end

# Pattern matching guards should not be flagged — they are part of case/in syntax
case element.name
in "a" if element.children.any? { |c| c.text? }
  extract(element)
end

case url
in _, ("facebook.com" | "fb.com"), username, *rest unless username.in?(["admin"])
  process(username)
end

# Tab-indented: modifier form with tab expansion exceeds MaxLineLength (120)
# 3 tabs = 3 bytes but visual width is 6 (with IndentationWidth 2)
# modifier_len = 6 + 59 + 1 + 2 + 1 + 53 = 122 > 120
			if ["SQ"].include?(params[:invoice_type]) && item_idd !=0
				invoiceDetails_quantity = getIssueEstimatedHoursXY(item_idd)
			end

# rubocop:disable for OTHER cops should be counted in modifier form length
# The comment carries over to the modifier form, making the line too long
if (log_state == 'newCall' && cause != 'forwarded') || log_to_comment == 'voicemail' # rubocop:disable Style/SoleNestedConditional
  log_done = false
end

# Single-line direct collection values are accepted by RuboCop
install_win(if parent then parent.path end, widgetname)

[if condition then value end]

{ x: if condition then value end }

# AllowURI: `scheme:` at end of line (RuboCop's URI regex matches scheme: without //)
logger.warn "#{self.class} posting to plaintext endpoint, which is insecure" if logger unless endpoint.to_s =~ /^https:/

# rubocop:disable Layout/LineLength — modifier form on a disabled line should not trigger "too long"
Lighthouse::SubmitCareerCounselingJob.trigger_failure_events(msg, claim) if Flipper.enabled?(:pcpg_trigger_action_needed_email) # rubocop:disable Layout/LineLength

# Block-level rubocop:disable Layout/LineLength — modifier form inside a disabled region
# rubocop:disable Layout/LineLength
pdf.text "\n<b>Veteran, claimant, or representative Email:</b>\n#{form_data.signing_appellant.email}\n", inline_format: true unless short_claimant_email?
# rubocop:enable Layout/LineLength

# Parenthesized body (begin_type? in RuboCop) — non_eligible_body
if condition
  (some_expression)
end

merged[index] = if first[index] || second[index]
  (first[index].to_i + second[index].to_i)
end

if block_given? then (c = yield(c)) end

# Body line has comment after semicolon — RuboCop's contains_comment? checks by line
if condition
  rxo = 8; # large data need skip of 8 bytes
end

# Comment after `then` on condition line makes modifier form too long (indented to exceed 120)
        if annotation.destroy then # don't need no blank annotations cluttering up the db, this is extra long text here
          puts "DELETED"
        end
