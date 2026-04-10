# Block if with empty line after multiline condition
if foo &&
   bar

  do_something
end

# Single line if condition
if foo && bar
  do_something
end

# Single line while condition
while condition
  do_something
end

# Block while with empty line after multiline condition
while multiline &&
   condition

  do_something
end

# Block until with empty line after multiline condition
until multiline ||
   condition

  do_something
end

# elsif with empty line after multiline condition
if condition
  do_something
elsif multiline &&
   condition

  do_something_else
end

# Modifier if with empty line after multiline condition
do_something if multiline &&
                condition

do_something_else

# Modifier if at last position (no right sibling) — no offense
def m
  do_something if multiline &&
                condition
end

# Modifier while at last position (begin form, no right sibling) — no offense
def m
  begin
    do_something
  end while multiline &&
        condition
end

# Modifier unless at top level with no right sibling — no offense
do_something unless multiline &&
                    condition

# Single line if at top level
do_something if condition

# case/when with empty line after multiline condition
case x
when foo,
    bar

  do_something
end

# case/when with single line condition
case x
when foo, bar
  do_something
end

# rescue with empty line after multiline exceptions
begin
  do_something
rescue FooError,
  BarError

  handle_error
end

# rescue with single line exceptions
begin
  do_something
rescue FooError
  handle_error
end

# Modifier if where keyword is at end of line but predicate is single-line
# RuboCop checks condition.multiline? (predicate first_line vs last_line)
raise ArgumentError, "bad index" if
  index > size && index < max

# Modifier unless where keyword is at end of line but predicate is single-line
do_something unless
  condition_met?

# Block if where keyword is at end of line but predicate is single-line
if
  some_condition
  do_something
end

# Ternary if — no offense even if condition is multiline (rare but possible)
x = (a &&
  b) ? 1 : 2

# Block if with whitespace-only line after multiline condition (treated as blank)
if helpers_data['x'] &&
   helpers_data['y'] &&
   helpers_data['z']

  puts "found"
end

# Modifier if with only comment after (no right sibling in AST)
def m
  true if depth >= 3 &&
          caller.first.label == name
          # TODO: incomplete
end

# Modifier unless inside when block — when is not a right sibling
case parent
when Step
  return render_403 unless can_read_module?(protocol) ||
                           can_read_repository?(protocol)
when Result
  return render_403 unless can_read_result?(parent)
end

# Block unless with single-line block as condition (block braces on same line)
# — the block { } is single-line, so condition is NOT multiline per RuboCop
unless %w[foo bar baz]
    .all? { |name| File.exist? File.join(path, name) }
  run("command")
end

# Block if with single-line block as condition — method chain spans lines
# but block { } is on one line
if items
     .reject { |la| la.value.nil? }
     .find { |la| la.value.length > 100 }
  report_error
end

# Block elsif with single-line block condition
if credentials
  use(credentials)
elsif credentials
      .class
      .ancestors
      .any? { |m| m.name == 'OAuth2::AccessToken' }
  use_token
end

# Modifier if with single-line block condition and right sibling
check(node, '@include') if node.children
                               .any? { |child| child.is_a?(Sass::Tree::Node) }
yield

# Block if with multiline array condition and single-line block
if %w[
  foo bar baz qux
].any? { |key| ENV[key].present? }
  report
end

# Block if with multiline array and single-line none? block
if [
    @host, @username, @password,
    @key_file, @session, @socket,
].none?{ |v| v != UNSET_VALUE }
  use_default
end

# Modifier while (non-begin form) at end of method — no right sibling
def optimize(code)
  code = code.dup
  nil while
    code.gsub!(/pattern/) { |f| f.upcase }
end

# Modifier if last of multiple statements before elsif — no right sibling
if valid_period
  posts_left = 0
  url = "#{view.topic.url}/2"
  reply_count = view.filtered_posts.count - 1
  reply_count = 0 if reply_count < 0
  posts_left = reply_count - limit if reply_count >
    limit
elsif embed_url.present?
  enqueue(:retrieve, user_id: current_user.try(:id))
end

# Modifier if last of multiple statements before else — no right sibling
if params[:archetype].present?
  args[:archetype] = params[:archetype]
  args[:participants] = params[:participants] if params[:participants].present? &&
    params[:archetype] == "private_message"
else
  args[:category_id] = params[:category_id].to_i if params[:category_id].present?
end

# Modifier if last of multiple statements before else (assignment form)
if opts[:private_message]
  scale_entropy = min_length.to_f / min_post_length.to_f
  entropy = (entropy * scale_entropy).to_i
  entropy =
    (min_length.to_f * ENTROPY_SCALE).to_i if entropy >
    min_length
else
  entropy = (min_post_length.to_f * ENTROPY_SCALE).to_i if entropy >
    min_post_length
end

# Modifier unless inside rescue handler before next rescue — no right sibling
begin
  constantize(word)
rescue NameError => e
  raise unless e.message =~ /(uninitialized constant|wrong constant name) #{const_regexp(word)}$/ ||
    e.name.to_s == word.to_s
rescue ArgumentError => e
  raise unless e.message =~ /not missing constant #{const_regexp(word)}\!$/
end

# Modifier if inside rescue handler before next rescue — no right sibling
def safe_constantize(word)
  constantize(word)
rescue NameError => e
  raise if e.name && !(word.to_s.split("::").include?(e.name.to_s) ||
    e.name.to_s == word.to_s)
rescue LoadError => e
  message = e.respond_to?(:original_message) ? e.original_message : e.message
  raise unless /Unable to autoload constant #{const_regexp(word)}/.match?(message)
end

# Modifier unless last of multiple statements before rescue — no right sibling
def validate_upload
  return if current_admin.present?
  head :unprocessable_entity unless [
    maximum_allowed_size.try(:to_i) >= blob_args[:byte_size].try(:to_i),
    content_types.any? { |pattern| pattern.match?(blob_args[:content_type]) },
    allowed_extensions.any? { |pattern| pattern.match?(extension) }
  ].all?
rescue NoMethodError
  head :unprocessable_entity
end

# Modifier unless with do..end block before rescue — no right sibling
def determine_validity(import_file_upload)
  widgets = YAML.load(import_file_upload.uploaded_content)
  raise InvalidWidgetYamlError unless widgets.all? do |widget_or_key, _|
    widget_or_key["MiqWidget"] || widget_or_key == "MiqWidget"
  end
rescue Psych::SyntaxError
  raise NonYamlError
end

# Modifier unless with multiline call args before else — no right sibling
def increment(key, amount = 1, options = {})
  backend.transaction do
    if existing = @load_for_update.call(key: key)
      existing_value = existing[config.value_column]
      amount += Integer(existing_value)
      raise IncrementError, "no update" unless @increment_update.call(
        key: key,
        value: existing_value,
        new_value: blob(amount.to_s)
      ) == 1
    else
      @create.call(key: key, value: blob(amount.to_s))
    end
  end
end

# Modifier unless before rescue (raise with line continuation)
raise "failed to create test zip" \
  unless system(
    "zip -q file.zip entry1 entry2"
  )
rescue StandardError
  puts "zip failed"

# Modifier if before rescue inside iterator — no right sibling
entries.each do |dev_path|
  dev = File.basename(dev_path)
  return id if (dev.start_with?("nvme") && id.include?("nvme-eui.")) ||
    (dev.start_with?("sd") && id.include?("wwn-")) ||
    (dev.start_with?("md") && id.include?("md-uuid-"))
rescue SystemCallError
  next
end

# Modifier if inside case/when before else — no right sibling
case access
when :any
  return true if group_through.klass.eager_load(:group).exists?(
    group_through.foreign_key => id,
    group_id: group,
    groups: {
      active: true
    }
  )
else
  return true if has_permission?(access)
end

# Modifier if inside block — scope closer `}` on condition line (no right sibling)
c.urls.find{|x|break x if
x.scan(p).size==g.size&&x.inject(x){|x,a|x.sub p,a}}

# Modifier if with }; closing enclosing scope on condition line (minified)
constants.map{|c|k=const_get(c);
k.meta_def(:urls){[f(k,p)]} if (!k
.respond_to?(:urls) || mu==true)};end end
X=Controllers

# Modifier if with multiline condition and trailing comment — not real content
return true if name =~ /\.map$/i ||  # Name convention
  lines[0] =~ /^{"version":\d+,/

next_statement

# Regular if with multiline condition and trailing comment followed by blank line
if lines[0] == '(function() {' &&     # First line is module closure opening
    lines[-2] == '}).call(this);' &&  # Second to last line closes module closure
    lines[-1] == ''                   # Last line is blank

  do_something
end

# next unless with multiline condition and trailing comment
items.each do |l|
  next unless basename =~ /^[^_]*_#{provider}_/ || # For first section
    basename =~ /^[^_]*_other_/

  process(l)
end

# next if with trailing comment on last condition line
items.each do |l|
  next if [
    :abstract_api,
    :twogis
  ].include?(l) # lookups that always return a result

  process(l)
end

# next if with line-continuation backslashes and blank line after condition
contents.each do |t|
  next if t == image \
          || t.vendor != image.vendor \
          || t.variant != image.variant \
          || t.arch != image.arch \
          || t.distribution != image.distribution \

  t.tags.delete_if { |tag| image.tags.include?(tag) }
end

# Modifier unless inside parenthesized expression — no right sibling in Parser AST
# because parens create a begin node wrapping the modifier
when "file" then
  mtime = (stat.mtime unless (last_modified = @last_modified[rel_file_name] and
                              stat.mtime.to_i <= last_modified.to_i))

  if force_doc or RDoc::Parser.can_parse(rel_file_name) then
    do_something
  end

# Modifier if inside parenthesized array element — closing `)` is not a right sibling
[
  root_item,
  (
    forecast_item if Setting.enable_forecast &&
      Sensor::Config.exists?(:inverter_power_forecast)
  ),
  essentials_item,
].compact

# next if with hash arg containing string "}" — multiline condition with blank line after
while rest?
  next if scan_block( {"("=>")", "do"=>"end", "begin"=>"end", "{"=>"}"},
                      /\s*(?:\.|::)\s*[a-zA-Z_]\w+\s*/ )

  next if scan_block( nil, /\s*/, {'{' => '}', '[' => ']'} )
end

# Block if with `then` keyword after multiline parenthesized condition — not real content
if (
  result.is_a?(Array) &&
  result[0].is_a?(Class) &&
  result[0].ancestors.include?(Exception)
) then

  do_something
end
