foo&.bar
foo&.bar&.baz
foo && foo.owner.nil?
foo && foo.empty?
foo && bar.baz
foo && foo < bar

# Dotless operator calls ([], []=, +, etc.) — safe nav not idiomatic
previous && previous['verified_at'].present?
obj && obj[:key].method_call
options && options[:codecs].include?(codec)
foo && foo[0].bar
foo && foo + bar

def min(rows, summary_column)
  rows && (rows.collect { |r| r[summary_column] }).min
end

# Ternary with [] operator — not idiomatic with safe nav
foo ? foo[index] : nil
foo ? foo[idx] = v : nil

# Ternary with nil? result (not safe nav pattern)
foo.nil? ? bar : baz

# Ternary with empty? — unsafe
foo.nil? ? nil : foo.empty?

# Ternary: foo ? nil : foo.bar — wrong direction
foo ? nil : foo.bar

# Methods that nil responds to in the chain — unsafe to convert
foo && foo.owner.is_a?(SomeClass)
foo && foo.value.respond_to?(:call)
foo && foo.name.kind_of?(String)
foo && foo.split.to_json
env["NODE_LABELS"] && env["NODE_LABELS"].split.to_json

# AllowedMethods (present?, blank?) in the chain
config && config.value.present?
foo && foo.bar.blank?
portal && portal.custom_domain.present?

# && inside assignment method call (e.g. []=) — unsafe context
cookies[token] = user && user.remember_me!
result[key] = obj && obj.value
foo.bar = baz && baz.qux

# && inside dotless method call arguments — unsafe context
# (RuboCop skips when ancestor send is dotless, e.g. scope, puts)
scope :accessible_to_user, ->(user) { user && user.name }
puts(foo && foo.bar)
(foo && foo.bar).to_s
foo && (foo.bar).to_s

# Negated wrappers make safe navigation unsafe
!!(foo && foo.bar)
obj.do_something if !obj

# Outer operator/assignment parents make modifier `if` unsafe
value - begin
  foo.bar if foo
end - used

hash[:categories] = begin
  foo.bar if foo
end

# && inside send/public_send arguments — RuboCop skips dynamic dispatch context
obj.send(:x, foo && foo.map { |h| h })
obj.public_send(:x, foo && foo.downcase)

# && inside `::` call arguments is skipped like RuboCop
BTC::Invariant(output && output.verified?, "message")

# Ternaries inside unsafe dotless call arguments are skipped
instance_variable_set("@foo", foo.nil? ? nil : foo.to_s)

# Chained && inside blocks keeps RuboCop's non-flattened traversal
items.each do |record_type|
  if dns_feasible?(record_type) && dns_record(record_type) && dns_record(record_type).conflicting?
    queue.create
  end
end

# Modifier if/unless inside call arguments or `private def` are skipped
install_win(if parent then parent.path end, widgetname)

private def foo(bar)
  bar.baz if bar
end

# Ternary inside dynamic send arguments is skipped
send "#{options[:foreign_key]}=", new_value ? new_value.send(options[:primary_key]) : nil

# Conditions already using `&.` are left alone
callback.call unless callback&.nil?

# Block-pass arguments are skipped like RuboCop
obj.public_send(@method, *@arguments, &(@block && @block.to_proc))
obj.public_send(:x, &(foo ? foo.bar : nil))

# If/ternary used as the receiver of another call are skipped
{ debug: (writer_opts[:debug].join("\n") if writer_opts[:debug]) }.to_json
"#{(model ? model.serial : nil).inspect}"

# Comparison operators after the checked receiver are skipped
if matcher && matcher === actual
  matcher
end

# Mixed `and` / `&&` chains are not flattened across precedence groups
raise Interrupt if status and status.signaled? && status.termsig == 1
foo && (foo.bar? || foo.baz?)
purchase && (purchase.stripe_refunded || (purchase.chargeback_date.present? && !purchase.chargeback_reversed))
corporation and (corporation.operated? or corporation_sold_out?(corporation))
foo && (foo.admin? || (foo == owner && foo.pro?))

# Block-receiver bodies that themselves end in block calls are skipped
items.map { options.queries && options.queries.keys.map { |q| q } }.compact.flatten
items.map { options.queries ? options.queries.keys.map { |q| q } : nil }.compact.flatten
items.map { options.queries.keys.map { |q| q } if options.queries }.compact.flatten

framework_input_paths.flat_map do |framework_path|
  outputs = unless framework_path.paths.nil?
              framework_path.paths.map do |path|
                path
              end
            end
  [*outputs]
end.compact.uniq

# Parenthesized lhs in `&&` is skipped like RuboCop
(safe_site['authentication']) && safe_site['authentication'].is_a?(Hash)

# Ternaries used as dotless operator receivers are skipped
(expected.nil? ? nil : expected.to_date) == actual

# Nested call-argument ternaries with block bodies are skipped
RbLazyFrame.new_from_parquet(
  sources,
  schema,
  ScanOptions.new(
    storage_options: storage_options ? storage_options.map { |k, v| [k.to_s, v.to_s] } : nil
  ),
  parallel
)

# Outer ruby2js `async` sends remain unsafe ancestors
async {
  def disconnect()
    @source.unpair(self) if @source
  end
}[]

async def run_cli
  comments_map.forEach do |value, key|
    node_desc = if key && key.type
      "s(:#{key.type}, ...)"
    else
      key.to_s[0, 50]
    end
  end
end

async def load
  setTimeout(5_000) { @pending.resolve() if @pending; @pending = nil }
end

# Lambda inside hash value of dotless call arguments — RuboCop
# walks all send ancestors and finds the dotless call as unsafe
before_save :inherit_restricted_status,
  if: -> { parent && parent.restricted? }

# Ternary inside nil-method call arguments — ancestor walk finds
# instance_variable_set as a nil-responding method
instance_variable_set("@bar", baz.nil? ? nil : baz.to_s)

# && chain length 1 inside block of dotless call args — RuboCop's
# ancestor walk escapes the rhs expression and finds the unsafe parent
puts(items.map { |x| x && x.b })
Hash[list.map { |p| [p.name.to_s, p.dt && p.dt.to_s] }]
scope :accessible, ->(u) { items.select { |i| i && i.active? } }

# Parenthesized condition in modifier if — RuboCop skips because ParenthesesNode
# wrapping causes find_matching_receiver_invocation to fail to match the receiver
obj.bar if (obj)
Plugin.dialog_manager.validate(X) if (Plugin.dialog_manager)
@cmd.set_validation_proc { Plugin.dialog_manager.validate(X) if (Plugin.dialog_manager) }

# && inside if-else body of assignment — unsafe parent crosses if branches
cookies[k] = if v && v.to_s
  v
else
  default
end

# Modifier-if inside block of dotless call — unsafe parent crosses block boundary
puts(items.map { |x| x.value if x })

# Modifier-unless inside block of dotless call — unsafe parent crosses block boundary
puts(items.map { |x| x.bar unless x.nil? })

# && inside else body of !!() — unsafe parent preserved across if branches
!!(if cached; cached; else; data && data.to_json; end)
