x = 1 +
    2

y = 3 + 4

z = a &&
    b

# Chained || on continuation line (both on same line = no offense)
def related_to_local_activity?
  fetch? || followed_by_local_accounts? || requested_through_relay? ||
    responds_to_followed_account? || addresses_local_accounts?
end

# Multiline block result + operator on same line
x = if true
  begin
    foo
  end + bar
end

# Nested && inside || (right operand of nested op aligned differently)
def acceptable?(node)
  src = node.source
  src.include?(QUOTE) &&
    (STRING_INTERPOLATION_REGEXP.match?(src) ||
    (node.str_type? && double_quotes_required?(src)))
end

# Leading operator style: && at start of continuation line
def regexp_first_argument?(send_node)
  send_node.first_argument&.regexp_type? \
    && REGEXP_ARGUMENT_METHODS.include?(send_node.method_name)
end

# Operations inside parentheses (grouped expressions) are not checked
if style != :either ||
   (start_loc.line == source_line_column[:line] &&
       start_loc.column == source_line_column[:column])
  do_something
end

# Method call with parenthesized args containing multiline op
!(method_name.start_with?(prefix) &&
    method_name.match?(/^foo/)) ||
  method_name == expected

# Operator inside method call arg list parentheses (not_for_this_cop?)
foo.permit(
  [completed_message: %i[title body]] +
                      [submitters: [%i[uuid]]]
)

# Operator inside .pick() parenthesized args
foo.pick(
  Arel::Nodes.build_quoted(Time.current) -
   Arel.sql("COALESCE(scheduled_at, created_at)")
)

# Boolean chain in hash value — operand-aligned in aligned style
data = {
  username: oauth.extra.try(:[], 'username').presence ||
            oauth.extra.try(:[], 'screen_name'),
  bio:      oauth.info.try(:[], 'description').presence ||
            oauth.extra.try(:[], 'bio').presence ||
            oauth.info.try(:[], 'headline')
}

# Chained + inside method call args (no parens) — RuboCop accepts via
# argument_in_method_call; we accept via preceding-comma heuristic
def from_string(str)
  raise Exception,
  "Unrecognizable input. " +
  "Please supply a folder, " +
  "filename, string or number."
end

# And/Or in keyword condition with double-width indentation
def find_key
  if (key_id = request.headers.fetch("KEY", "").presence) &&
     (signature = request.headers.fetch("SIG", "").presence)
    use_key(key_id, signature)
  end
end
