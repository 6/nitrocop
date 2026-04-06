def foo
  return if need_return?

  bar
end

def baz
  return if something?
  return if something_different?

  bar
end

def quux
  raise "error" unless valid?

  do_work
end

def last_guard
  return if done?
end

def consecutive_with_embedded_return
  return if does_not_expire?
  requeue! && return if not_due_yet?

  notify_remote_voters_and_owner!
end

def consecutive_mixed_guards
  raise "error" unless valid?
  do_something || return if stale?

  process
end

# Comment between consecutive guard clauses is OK
def comment_between_guards
  return if first_condition?
  # This is a comment explaining the next guard
  return if second_condition?

  do_work
end

# Multiple comments between guards
def multi_comment_between_guards
  return unless valid_input?
  # First reason
  # Second reason
  return if already_processed?

  process
end

# Guard followed by multi-line if block containing return
def guard_then_multiline_if
  return if done?
  if complex_condition? && another_check?
    return
  end

  process
end

# Guard followed by multi-line unless block containing raise
def guard_then_multiline_unless
  return unless valid?
  unless authorized? && permitted?
    raise "unauthorized"
  end

  do_work
end

# Guard inside a block (embedded in larger expression)
def guard_in_block
  heredocs.each { |node, r| return node if r.include?(line_number) }
  nil
end

# Break inside a block (embedded in larger expression)
def break_in_block
  prev = items.each_cons(2) { |m, n| break m if n == item }
  between = prev.source_range
end

# Guard before end keyword
def guard_before_end
  return if something?
end

# Guard before else
def guard_before_else(x)
  if x > 0
    return if done?
  else
    work
  end
end

# Guard before rescue
def guard_before_rescue
  return if safe?
rescue => e
  handle(e)
end

# Guard before ensure
def guard_before_ensure
  return if cached?
ensure
  cleanup
end

# Guard before when
def guard_before_when(x)
  case x
  when :a
    return if skip?
  when :b
    work
  end
end

# Next in block followed by comment then next
def next_with_comments
  items.each do |item|
    next if item.blank?
    # Skip already processed items
    next if item.processed?

    process(item)
  end
end

# Raise with comment between guards
def raise_with_comments
  raise "error" unless condition_a?
  # Make sure condition_b is also met
  raise "other error" unless condition_b?

  run
end

# Guard with nocov directive followed by blank line
def guard_with_nocov
  # :nocov:
  return if condition?
  # :nocov:

  bar
end

# Guard clause is last statement before closing brace
def guard_before_closing_brace
  items.map do |item|
    return item if item.valid?
  end
end

# Guard followed by multiline if with return inside nested structure
def guard_then_nested_multiline_if
  return if line_length(line) <= max
  return if allowed_line?(line)
  if complex_config? && special_mode?
    return check_special(line)
  end

  register_offense(line)
end

# Multiple return false guards with comments
def multiple_return_false_guards
  return false unless first_check?
  # anonymous forwarding
  return true if special_case?
  return false unless second_check?
  return false unless third_check?

  name == value
end

# Guard with long comment block between guards
def long_comment_between_guards
  return false unless incoming? || outgoing?
  # For Chatwoot Cloud:
  #   - Enable indexing only if the account is paid.
  #   - The `advanced_search_indexing` feature flag is used only in the cloud.
  #
  # For Self-hosted:
  #   - Adding an extra feature flag here would cause confusion.
  return false if cloud? && !feature_enabled?('search')

  true
end

# Block-form if with guard clause followed by empty line — no offense
def block_guard_with_blank
  if params.blank?
    fail ParamsError, "Missing params"
  end

  process(params)
end

# Block-form if with guard clause at end of method — no offense
def block_guard_at_end
  if invalid?
    raise "invalid"
  end
end

# Block-form if with multiple statements — not a guard clause
def block_not_guard
  if condition?
    setup
    process
  end
  finalize
end

# Block-form if with multi-line guard statement — not a guard clause per RuboCop
# (guard_clause? requires single_line?)
def multiline_guard_in_block
  all.to_a.delete_if do |version|
    if item.respond_to?(:access)
      next item.user_id != user.id &&
        item.assigned_to != user.id &&
        (item.access == "Private")
    end
    next false
  end
end

# `and return` guard clause properly followed by blank line
def and_return_ok
  render :foo and return if condition

  do_something
end

# `or return` guard clause properly followed by blank line
def or_return_ok
  render :foo or return if condition

  do_something
end

# Guard with rubocop:enable followed by blank line
def guard_rubocop_enable_ok
  # rubocop:disable Department/Cop
  return if condition
  # rubocop:enable Department/Cop

  bar
end

# FP fix: spaced rubocop: enable directive is allowed too
def guard_spaced_rubocop_enable_ok
  # rubocop: disable Style/CaseEquality
  return if cond
  # rubocop: enable Style/CaseEquality

  work
end

# Multiple statements on same line with semicolon
def foo(item)
  return unless item.positive?; item * 2
end

# Guard before begin with blank line
def guard_before_begin_ok
  return another_object if something_different?

  begin
    bar
  rescue SomeException
    baz
  end
end

# Non-guard modifier if (not a guard clause)
def normal_modifier_if
  foo += 1 if need_add?
  foobar
end

# Guard clause with heredoc argument followed by blank line
def guard_heredoc_ok
  raise ArgumentError, <<-MSG unless path
    Must be called with mount point
  MSG

  bar
end

# Guard clause with squiggly heredoc followed by blank line
def guard_squiggly_heredoc_ok
  raise ArgumentError, <<~MSG unless path
    Must be called with mount point
  MSG

  bar
end

# Guard clause with heredoc in condition followed by blank line
def guard_heredoc_condition_ok
  return true if <<~TEXT.length > bar
    hi
  TEXT

  false
end

# Guard clause with heredoc and chained calls
def guard_heredoc_chained_ok
  raise ArgumentError, <<~END.squish.it.good unless guard
    A multiline message
    that will be squished.
  END

  return_value
end

# Ternary without guard clause - not flagged
def ternary_non_guard
  x = condition ? value_a : value_b
  do_something
end

# Guard clause followed by whitespace-only blank line (spaces)
# RuboCop treats whitespace-only lines as blank
def guard_whitespace_blank_spaces
  return false unless request&.fullpath&.start_with?(callback_path)
      
  # Try request.origin first, then fallback to referer.
  origin = request.origin
end

# Guard clause followed by whitespace-only blank line (tab)
def guard_whitespace_blank_tab
  raise ActiveRecord::RecordNotFound unless record.present?

  process(record)
end

# Consecutive guard clauses with line continuation (backslash)
def consecutive_guards_with_continuation
  raise ArgumentError, "invalid method" \
    unless method == 'dns'
  raise ArgumentError, "a non-empty list is required" \
    if servers.empty?
end

# Multiple consecutive guards with line continuation
def multiple_guards_continuation
  raise ArgumentError, "method should be a symbol" \
    unless method.is_a?(Symbol)
  raise ArgumentError, "uri should be a string" \
    unless uri.is_a?(String)
  raise ArgumentError, "body should be a string" \
    if body && !body.is_a?(String)
  raise ArgumentError, "headers should be a hash" \
    if headers && !headers.is_a?(Hash)
end

# Guard with line continuation followed by non-guard with blank line
def guard_continuation_then_blank
  raise ArgumentError, "invalid input" \
    unless valid?

  process
end

# Guard with line continuation at end of method
def guard_continuation_at_end
  raise ArgumentError, "missing config" \
    unless config.present?
end

# Guard with string concatenation continuation
def guard_string_concat_continuation
  raise "Must specify the file to " + \
    "convert to the new model" if filename.nil?
  raise "File does not " + \
    "exist: #{filename}" unless File.exist?(filename)
end

# Guard with multi-line return value followed by modifier
def guard_multiline_return_value
  return {
    status: "err",
    error: "Invalid input."
  }.to_json if !info
  return {
    status: "err",
    error: "Wrong ID."
  }.to_json if not get_item(id)
end

# Guard with multi-line return string followed by code
def guard_multiline_return_string
  return "
    * navigate
  " if options[:task] == []
  prefix = "open" if options[:task] == ["navigate"]
end

# Guard with multi-line raise (args on next line)
def guard_multiline_raise_args
  raise ArgumentError,
    "msg here" unless condition
  raise BadError,
    "Response is empty." if raw_text.blank?
end

# Guard with fail and line continuation
def guard_fail_continuation
  fail "Association defined for a second time. " \
       "Associations can only be defined once" if duplicate?(name)
  associations[name] = object
end

# Guard with parenthesized multi-line condition
def guard_paren_multiline_condition
  raise ArgumentError, "invalid interval" if (
      discovery.key?('interval') &&
      !(discovery['interval'].is_a?(Numeric) &&
      discovery['interval'] >= 0)
    )
  raise ArgumentError, "missing host" \
    unless discovery['hosts']
end

# Guard followed by comment, blank line, then another guard
def guard_comment_blank_guard
  next if file =~ /pattern_a/ && VERSION <= Gem::Version.new('1.7.25')
  # EMXIF

  # FIXME: Remove when we stop testing old version
  next if file =~ /pattern_b/ && VERSION <= Gem::Version.new('1.7.13')
end

# FP fix: Guard followed by ternary with guard in if-branch
def guard_then_ternary_guard
  return unless broken_rule
  fail_build ? fail(message) : warn(message)
end

# FP fix: Guard followed by ternary with break/next
def guard_then_ternary_break_next
  items.each do |item|
    next unless item.check_port
    item.run || error ? break : next
  end
end

# FP fix: Guard followed by comment then blank then if-block with guard
def guard_comment_blank_if_guard
  return true if result
  # comment about the next check
  # more details

  if BCrypt::Password.new(enc) == [password].join
    return true
  end
end

# FP fix: Block guard followed by if-block with `&& return`
def block_guard_then_and_return
  unless @work
    raise "not found"
  end
  if @collection
    redirect_to(@work) && return
  end
end

# Guard before if-block with single-line raise (IS a guard clause)
def guard_then_if_single_line_raise
  return if !argv
  if argv.empty?
    raise "error"
  end
end

# Multi-line raise guard continuation with parens in condition
def multiline_raise_continuation_parens
  raise "failed to create test zip" \
    unless system("zip -q test.zip test/data/file.txt")
  raise "failed to remove entry" \
    unless system(
      "zip -q test.zip -d test/data/file.txt"
    )
end

# FP fix: Guard followed by multi-line if block with guard (condition spans multiple lines)
# RuboCop: return if guards → next sibling is block-form if with guard → no offense
def guard_then_multiline_cond_if_guard
  return unless active?
  return if status != "regular" || topic.private?
  return if pending.where(target: post).exists?
  if created_by.bot? || created_by.staff? ||
       created_by.has_trust_level?(4)
    return
  end
end

# FP fix: Guard followed by multi-line if with two-line condition containing guard
def guard_then_two_line_cond_guard
  return if value == "f"
  if config.email.present? ||
       config.address.present?
    return
  end
end

# FP fix: Guard followed by multi-line unless with guard
def guard_then_multiline_unless_guard
  next unless flags.include?(d)
  unless records.nil?
    raise ArgumentError, "Only one option allowed"
  end
end

# FP fix: Block-form if with guard followed by another block-form if with guard
# (both are consecutive guard clauses — RuboCop doesn't flag)
def consecutive_block_form_guards
  if is_admin? && groups.all? { |g| g.level == 1 }
    return true
  end
  if is_staff? && groups.all? { |g| g.level == 2 }
    return true
  end
  if authenticated? &&
       groups.all? { |g| g.level == 3 }
    return true
  end
end

# FP fix: Guard followed by if block with multi-line condition (parens, &&)
def guard_then_multiline_paren_condition
  return nil if value.blank?
  if defined?(ActiveRecord) && value.is_a?(ActiveRecord::Base) &&
     value.respond_to?(:id) && value.id.is_a?(Integer)
    return value.id
  end
end

# FP fix: Guard followed by multi-line if block with raise guard
def guard_then_multiline_raise_guard
  raise "invalid config" unless store.is_a?(Base)
  if api_key && store.class.to_s != "WebServiceStore"
    raise "invalid configuration: only service expects an API Key"
  end
  if ENV["REDIS_URL"] &&
      defined?(::Adapters::WebServiceStore) &&
      store.instance_of?(::Adapters::WebServiceStore)
    raise "invalid configuration: service shouldn't have redis url set"
  end
end

# FP fix: Guard at end of block-form if, followed by multi-line if block with guard
def block_guard_end_then_multiline_if_guard
  if security.user_auth && security.users.empty?
    raise ConfigError, "users required"
  end
  if !security.allow_anon && security.clients.empty?
    raise ConfigError, "clients required"
  end
end

# FP fix: Modifier guard followed by if with condition spanning 4+ lines with guard
def guard_then_long_multiline_condition
  return true if ((old.ip != ip) ||
    (old.hostname != hostname) ||
    provision_changed? ||
    (old.subnet != subnet) ||
    (old.ip6 != ip6) ||
    (old.subnet6 != subnet6))
  if (is_a?(Nic::Base) && rebuild? &&
       !dhcp_record.valid?)
    return true
  end
end

# FP fix: block guard `break` inside `if (m == l)` then another multi-line if
def block_break_then_multiline_if
  if (m == l)
    break
  end
  if (@h[m][m-1].abs * (q.abs + r.abs) <
    eps * (p.abs * (@h[m-1][m-1].abs + z.abs +
    @h[m+1][m+1].abs)))
    break
  end
end

# FP fix: Guard followed by multi-line if with `and` operator and return
def block_guard_then_and_return_if
  unless @work
    raise "not found"
  end
  if @collection && has_access?
    redirect_to(@work) && return
  end
  if @alternate &&
     visible?
    redirect_to(@alternate) && return
  end
end

# FP fix: guard then multi-line if with === operator continuation
def guard_then_triple_equals_if
  return if disabled?
  if SomeClass ===
       (
         begin
           @message.message
         rescue StandardError
           nil
         end
       )
    return
  end
  return skip(reason) if @message.blank?
end

# FP fix: guard then multi-line if with return at end of method
def guard_then_multiline_cond_at_end
  return unless active?
  return if status != "regular"
  if condition_a || condition_b ||
       condition_c
    return
  end
end

# FP fix: block guard `end` followed by another block `if..raise..end` where
# the if condition contains braces from a block literal (e.g., `.all? { }`)
def block_guard_then_if_with_block_literal
  if @optional_argument
    raise ArgumentError, "Options not supported"
  end
  if @optional_argument and !@opts.all? { |o| o =~ /[ =]\[/ }
    raise ArgumentError, "Option is inconsistent"
  end
end

# FP fix: guard followed by `unless..raise..end` with multi-line `or` condition
def guard_then_unless_with_or_continuation
  return [n, "unexpected format: #{lhs}"] if lhs_name.nil?
  unless @instance.tables.has_key? lhs_name.to_sym or
         @instance.lattices.has_key? lhs_name.to_sym
    return [n, "Collection does not exist: '#{lhs_name}'"]
  end
end

# FP fix: guard followed by `if..return..end` where condition contains regex
def guard_then_if_with_regex_condition
  return "percona-toolkit" if query =~ %r#\*\w+\.\w+:[0-9]/[0-9]\*/#
  if match = /\A\s*(call\s+\S+)\(/i.match(query)
    return match.captures.first.downcase!
  end
end

# FP fix: guard followed by multi-line `if` with `and` keyword in condition
# and block braces that confuse paren depth
def guard_then_if_with_and_continuation
  return doc.length if doc.cursor_offset == doc.length - 1
  if doc.length >= doc.cursor_offset + doc.delim.length and
      doc.get_range(doc.cursor_offset, doc.delim.length) == doc.delim
    return doc.cursor_offset + doc.delim.length
  end
end

# FP fix: block guard `end` followed by another block `if..return..end`
# where the next if condition uses comparison operators and is single-line
def consecutive_block_guards_single_line_cond
  if security.user_auth && security.users.empty?
    raise ConfigError, "users required"
  end
  if !security.allow_anon && security.clients.empty?
    raise ConfigError, "clients required"
  end
end

# FP fix: guard followed by `if..raise..end` where the raise message
# contains the word `if` inside a string literal
def guard_then_if_raise_with_if_in_string
  raise ArgumentError, "Must specify at least one column" if columns.empty?
  if relation.joins_values.present? && !@columns.all? { |column| column.to_s.include?(".") }
    raise ArgumentError, "You need to specify fully-qualified columns if you join a table"
  end
end

# FP fix: next guard followed by `unless..raise..end` (non-if keyword guard block)
def next_guard_then_unless_raise_block
  DELIMITERS.each do |d|
    next unless flags.include?(d)
    unless @delimiters.nil?
      raise ArgumentError, "Only one delimiter allowed"
    end
  end
end

# FP fix: block guard followed by if block where the if body is a return
# with the word `if` appearing inside a string argument
def guard_then_if_with_if_in_return_string
  if relation.orders.present?
    raise ConditionNotSupportedError
  end
  if relation.arel.orders.present? || relation.arel.taken.present?
    raise ConditionNotSupportedError
  end
end

# FP fix: block guard followed by another guard block whose condition continues
# onto the next line with a comparison operator
def attachments_too_large?(upload, optimized_1x, max_size)
  if (
    !upload.secure? && !stripped_upload_shas.include?(upload.sha1) &&
      !stripped_upload_shas.include?(optimized_1x&.sha1)
  )
    return
  end
  if (optimized_1x&.filesize || upload.filesize) >
       max_size
    return
  end

  true
end

# FP fix: next guard followed by `unless..raise..end` where the raise string
# contains bracket characters that should not affect guard-block detection
def delimiters_from(flags, format)
  found = nil
  DELIMITERS.each do |delimiter|
    next unless flags.include?(delimiter)
    unless found.nil?
      raise ArgumentError, "Only one of [ { ( < | can be given in #{format}"
    end

    found = delimiter
  end
end

# if/else with guard in if-branch is NOT a guard clause
def get(key, raise_error: false, raw: false)
  ret = if raise_error
          @storage.get(key) or raise UnknownKey.new("doesn't exist")
        else
          @storage.get(key)
        end
  if raw
    ret
  else
    ret && build_response(ret)
  end
end

# unless/else where the else body is a guard — NOT flagged because
# RuboCop's if_branch for unless is the unless body, not the else body
def unless_else_guard_in_else
  unless cond
    do_thing
  else
    raise "error"
  end
  next_code
end

# unless/else with guard in body, followed by blank line — no offense
def unless_else_guard_blank
  unless cond
    raise "error"
  else
    do_thing
  end

  next_code
end

# unless/else at end of method — no offense (last stmt)
def unless_else_guard_at_end
  unless cond
    raise "error"
  else
    do_thing
  end
end

# FN fix: block-form guard followed by non-guard if block with multiline raise string
def block_guard_then_if_multiline_string_raise(connect_string)
  if GitRepository.repository_exists?(connect_string)
    raise RepositoryCollision, "There is already a repository at #{connect_string}"
  end

  if File.exist?(connect_string)
    raise IOError, "Could not create a repository at #{connect_string}: some directory with same name exists
                         already"
  end
end

# FN fix: next sibling uses rescue modifier, so it is not itself a guard clause
def rating_average
  return self.rating_avg if attributes.has_key?('rating_avg')

  return (rating_statistic.rating_avg || 0) rescue 0 if acts_as_rated_options[:stats_class]
  avg
end

# FN fix: rescue modifier around the next line should not suppress the offense
def determine_lease_type
  return nil if group.nil?

  return "ip" if IPAddr.new(group) rescue false
  return "local" if Admin::Group.exists? group

  return "external"
end

# FN fix: ternary guard detection must ignore ternaries nested inside an if condition
def guard_then_if_with_ternary_break_in_condition(remaining)
  return if remaining.empty?

  if remaining.find { |n| (type = n.type) == :blank ? nil : ((BLOCK_TYPES.include? type) ? true : break) }
    el.options[:compound] = true
  end
end

# FN fix: comment text containing `if` must not make a bare return look like a guard
def output_extension(mime)
  return '.css' if mime.eql? 'text/css'

  return '.html' # if all else falls trough
end

# FP fix: interpolation inside a percent string is not an inline comment
def inline_link_substitution(text)
  text.gsub InlineLinkRx do
    if $2 && !$5
      next $&.slice 1, $&.length if $1.start_with? RS
      next %(#{$1}#{$&.slice $1.length + 1, $&.length}) if $3.start_with? RS
      next $& unless $6
    end
  end
end

# FP fix: `#{...}` inside a percent string must not break block-guard detection
def validate_processor(kind_name, block, processor)
  unless (name = as_symbol processor.name)
    raise ::ArgumentError, %(No name specified for #{kind_name} extension at #{block.source_location.join ':'})
  end
  unless processor.process_block_given?
    raise ::NoMethodError, %(No block specified to process #{kind_name} extension at #{block.source_location.join ':'})
  end
end

# FP fix: code after `end` can still belong to the same guard node (`end if ...`)
def parser_comment_guard(normal, next_line, reader, document, attributes)
  if normal && next_line.start_with? '.'
    return true
  elsif !normal || (next_line.start_with? '/')
    if next_line == '//'
      return true
    elsif normal && (uniform? next_line, '/', (ll = next_line.length))
      unless ll == 3
        reader.read_lines_until terminator: next_line, skip_first_line: true, preserve_last_line: true, skip_processing: true, context: :comment
        return true
      end
    else
      return true unless next_line.start_with? '///'
    end if next_line.start_with? '//'
  elsif normal && (next_line.start_with? ':') && AttributeEntryRx =~ next_line
    process_attribute_entry reader, document, attributes, $~
    return true
  end
end

# FP fix: nested modifier form is not itself a guard clause
def nested_modifier_not_guard(path, replace)
  return if File.exist?(path) unless replace
  work
end

# Guard with trailing semicolon followed by blank line
def guard_semicolon_with_blank
  return unless driver =~ /mysql/i;

  migrate!
end

# Guard with semicolon + comment followed by blank line
def guard_semicolon_comment_blank
  return "" if ex_obj == nil; # canceled

  ex_obj.cb_call
end


# Guard then if-block with bare `and return` (no modifier) — IS a guard, no offense
def guard_then_if_with_bare_and_return
  return if c.nil?
  if c.closed?
    do_thing and return
  end
end

# FP fix: block guard used as the value of a multiline assignment is embedded
def embedded_assignment_guard
  value =
    if cond
      return 1
    end
  work(value)
end

# FP fix: multiline embedded guard inside an inline block is not standalone
def inline_block_multiline_guard(items, cond)
  items.each { |item| return true unless cond.call(item,
    :extra) }
  work
end

# FP fix: embedded multiline guard ending with a closing delimiter can still
# carry an inline comment without becoming standalone code
def inline_block_multiline_guard_with_comment(items, cond)
  items.each { |item| return true unless cond.call(item,
    :extra) } # trailing comment
  work
end

# FP fix: semicolons inside nested parens do not split a guard line
def nested_semicolon_guard
  return @log if @log
  return (@log = (stub = Object.new; stub)) if @args[:nolog]
  return :fallback if default?

  work
end

# FP fix: block guard with attached rescue modifier is not followed by a sibling
def block_guard_with_rescue_modifier(x)
  if x.nan? || x.infinite?
    return 'null'
  end rescue nil
  x.to_s
end

# FP fix: `@if` ivar is not a modifier keyword in the next consecutive guard
def consecutive_guards_with_if_ivar(store)
  CALL_GRAPH.dup.each do |base_mod, mixins|
    base_mod_name = Utils.module_name(base_mod) or next
    content = store[base_mod_name]
    mixins.each do |(how, mod)|
      mod_name = Utils.module_name(mod) or next
      next if mod_name.start_with?("Orthoses")
      next unless @if.nil? || @if.call(base_mod, how, mod)

      store[mod_name].header = "module #{mod_name}"
      content << "#{how} #{mod_name}#{type_params_sig(mod)}"
    end
  end
end

# FP fix: same-line brace-block body with guard followed by `;` sibling
partially_sorted = (1..5).map{|i|
  ary = [5, 4, 3, 2, 1]
  ary.sort_by!{|x,y| break if x==i; x<=>y}
  ary
}

# FP fix: consecutive block-form guards where raise uses %{} with apostrophe
# count_bracket_depth_change must handle percent literals
def consecutive_block_guards_with_percent_string
  if cond1
    raise "simple error"
  end
  if cond2
    raise %{you'l need to pass a identifier}
  end
end

# FP fix: ternary guard with heredoc argument as last statement
def ternary_guard_with_heredoc_last_stmt
  searchables = filters.select(&:searchable?)
  searchables.blank? ? raise(<<-NO_SEARCHABLE_FILTERS) : searchables
    No searchable filters for unroll.
  NO_SEARCHABLE_FILTERS
end

# FP fix: consecutive modifier guards where next guard has regex with escaped quotes
def consecutive_guards_with_regex_escaped_quotes
  return 1 if key == 'a'
  return key.gsub!(/\A\'|\'\Z/, '') if key.start_with?("'") && key.end_with?("'")

  key.process
end

# FP fix: consecutive block-form guards where next if condition has inline comment
# containing parentheses and slashes
def consecutive_block_guards_inline_comment_parens
  if track_thing.created_at >= result[:model].described_at
    next
  end
  if result[:model].described_at < one_week_ago  # older than 1 week (see 14 days / 7 days in comment above)
    next
  end
  if done_info_request_events.include?(result[:model].id)
    next
  end

  work
end

# FP fix: guard with backslash continuation where trailing \ should not be
# treated as code after the guard
def guard_with_trailing_backslash_continuation
  contents.each do |t|
    next if t == image \
            || t.vendor != image.vendor \
            || t.variant != image.variant \
            || t.arch != image.arch \
            || t.distribution != image.distribution \

    t.tags.delete_if { |tag| image.tags.include?(tag) }
  end
end

# FP fix: consecutive guards where next sibling has `/` division operator
# The `/` preceded by a space was misidentified as a regex delimiter start,
# hiding the `if` keyword from the modifier count.
def scale_bytes_base2(amount)
  amount = amount.to_f
  return [ 0, "b" ] if amount == 0
  return [ amount.to_i, "b" ] if amount < @@kib
  return [ (amount / @@kib).round(1), "KiB" ] if amount < @@mib
  return [ (amount / @@mib).round(2), "MiB" ] if amount < @@gib
  return [ (amount / @@gib).round(3), "GiB" ] if amount < @@tib
  return [ (amount / @@tib).round(4), "TiB" ] if amount < @@pib

  return [ (amount / @@pib).round(5), "PiB" ]
end

# FP fix: consecutive guards with division in condition
def hue_to_rgb(p, q, t)
  t += 1 if (t < 0)
  t -= 1 if (t > 1)
  return(p + (q - p) * 6 * t) if (t < 1 / 6.0)
  return q if (t < 1 / 2.0)
  return(p + (q - p) * (2 / 3.0 - t) * 6) if (t < 2 / 3.0)

  p
end

# FP fix: consecutive guards where next sibling has `#{}` interpolation
# that breaks double-quote tracking in the scanner
def validate_entry(entry, file_n, zip)
  return "file '#{entry.name}' cannot be a #{entry.ftype}" if entry.ftype != :file
  return "file '#{entry.name}' has flags (#{"%.8b" % entry.gp_flags})" if entry.gp_flags != 0
  return "file has bad size" if entry.size != zip.entries[file_n].size
end
