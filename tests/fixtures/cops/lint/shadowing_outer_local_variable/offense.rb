def some_method
  foo = 1
  puts foo
  1.times do |foo|
              ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `foo`.
  end
end
def other_method
  foo = 1
  puts foo
  1.times do |i; foo|
                 ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `foo`.
    puts foo
  end
end
def method_arg(foo)
  1.times do |foo|
              ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `foo`.
  end
end
# Nested block: inner block param shadows outer block param
def nested_shadow
  items.each do |slug|
    slug.children.map! { |slug| slug.upcase }
                          ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `slug`.
  end
end
# Destructured block param shadows method arg
def theme_svgs(theme_id)
  sprites.map do |(theme_id, upload_id)|
                   ^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `theme_id`.
    [theme_id, upload_id]
  end
end
# Block inside if still shadows outer method arg
def some_method(env)
  if some_condition
    pages.each do |env|
                   ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `env`.
      do_something(env)
    end
  end
end
# Block param shadowing inside if/unless branch still flags
def handler(name)
  if block_given?
    items.each do |name|
                   ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `name`.
      yield name
    end
  end
end
# Same branch of same if condition node
def some_method
  if condition?
    foo = 1
    puts foo
    bar.each do |foo|
                 ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `foo`.
    end
  else
    bar.each do |foo|
    end
  end
end
# Splat block param shadows outer
def some_method
  foo = 1
  puts foo
  1.times do |*foo|
              ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `foo`.
  end
end
# Block block param shadows outer
def some_method
  foo = 1
  puts foo
  proc_taking_block = proc do |&foo|
                               ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `foo`.
  end
  proc_taking_block.call do
  end
end

# Block param inside def self body still shadows that method's own param
class Client
  def self.search(options)
    Options.propagate_important_options(options) do |options|
                                                     ^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `options`.
      options
    end
  end
end

# Post parameter shadows in inner block
def configure(*items, tail)
  jobs.each do |tail|
                ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `tail`.
    puts tail
  end
end

# Keyword rest parameter shadows in inner block
def configure(**options)
  handler = proc do |**options|
                     ^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `options`.
    options
  end
  handler.call
end

# FN fix: variable in non-adjacent elsif branches (2+ branches apart)
def magic_method(method)
  if method =~ /^items$/
    items
  elsif method =~ /^first_item$/
    e = find_item(method)
    e ? e[0] : nil
  elsif method =~ /^parent_item$/
    find_parent(method)
  elsif method =~ /^each_item$/
    each_entity(method) { |e| yield e }
                           ^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `e`.
  end
end

# FN fix: variable from while loop, block in else of same if
def compress(body)
  if body.is_a?(::File)
    while part = body.read(8192)
      write(part)
    end
  else
    body.each { |part|
                 ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `part`.
      write(part)
    }
  end
end

# FN fix: block param shadows outer from nested block in same scope
def build_graph(prev)
  block.prev.each do |prev|
                      ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `prev`.
    trans[prev]
  end
end

# FN fix: elsif condition assignment, block in later elsif shadows earlier
def validate_archive(archive)
  if archive.too_large?
    report_error
  elsif entry = archive.entries.find { |entry| entry.starts_with?("/") }
    report(entry)
  elsif entry = archive.entries.find { |entry| entry.traversal? }
                                        ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `entry`.
    report(entry)
  end
end


# FN fix: variable from block, block param inside block body shadows it
def process_items(times)
  times_by_group.each do |group, times|
                                 ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `times`.
    times.each { |t| group.enqueue(t) }
  end
end

# FN fix: variable from method arg, block in else branch shadows it
def handle(response)
  if responses.length == 1
    run(response)
  elsif responses.length > 1
    responses.each_with_index do |response, index|
                                  ^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `response`.
      say response[:command]
    end
  end
end

# FN fix: variable in if-branch, block in multi-statement elsif branch
def build_graph
  if items.size == 1
    prev = items.first
    use(prev)
  elsif items.size > 1
    names = items.map(&:name)
    items.each do |prev|
                   ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `prev`.
      process(prev)
    end
  end
end

# FN fix: variable in case/when, block in different multi-statement when
def run_server(engine)
  case engine
  when "puma"
    server = create_puma
    server.run.join
  when "thin"
    handler = get_handler("thin")
    handler.run(app) do |server|
                         ^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `server`.
      server.ssl = true
    end
  end
end

# FN fix: splat rest param inside destructured block param shadows outer
def join_results(fruits)
  actual.map { |(car, *fruits)| [car, fruits.map(&:name)] }
                       ^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `fruits`.
end

# FN fix: when-condition assignment in second when clause shadows first when's var
def transform(decls)
  case
  when decl = decls.find {|decl| decl.special? }
    process(decl)
  when decl = decls.find {|decl| decl.lambda? }
                           ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `decl`.
    transform(decl)
  end
end

# FN fix: variable assigned earlier, block param in find on separate line
def locate(tp, caller_locations)
  loc = build_source_location(tp, caller_locations)
  caller_location = caller_locations
    .find { |loc| loc.path && File.exist?(loc.path) }
             ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `loc`.
  caller_location
end

# FN fix: multi-assign LHS variable, block in else branch shadows it
def find_source(accounts)
  host, username, password = accounts.find { |h, u, p| h == target }
  if username
    use(host)
  else
    accounts.each do |host, olduser, oldpw|
                      ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `host`.
      menu.choice(olduser, host)
    end
  end
end

# FN fix: block param shadows variable from outer catch/else scope
def parse_args(sw)
  catch(:prune) do
    visit(:each_option) do |sw|
                            ^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `sw`.
      sw.block.call(arg) if Switch === sw
    end
  end
end

# FN fix: if/else — block nested in method chain in else body, var in if body
def track_constant(tp, caller_locations)
  if File.exist?(tp.path)
    loc = build_source_location(tp, caller_locations)
  else
    caller_location = caller_locations
      .find { |loc| loc.path && File.exist?(loc.path) }
               ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `loc`.
    loc = resolve_location(caller_location)
  end
end

# FN fix: variable reassigned inside block scope (case branch), block in else
def parse_in_order(argv, setter)
  opt, arg, sw, val, rest = nil
  catch(:terminate) {
    while arg = argv.shift
      case arg
      when /\A--/
        sw, = complete(:long, opt, true)
      else
        catch(:prune) do
          visit(:each_option) do |sw|
                                  ^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `sw`.
            sw.block.call(arg)
          end
        end
      end
    end
  }
end

# FN fix: adjacent elsif — block nested in method chain, not direct branch child
def schema_example(value)
  if value.key?("oneOf")
    value["oneOf"].first
  elsif value.key?("anyOf")
    ref = value["anyOf"].first
    schema_example(ref)
  elsif value.key?("allOf")
    value["allOf"].map { |ref| schema_example(ref) }.reduce({}, &:merge)
                          ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `ref`.
  end
end

# FN fix: Thread.new with splat args — not suppressed (only Ractor.new is special)
def start_thread(*args)
  Thread.new(*args) { |*args| process(*args) }
                       ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `args`.
end

# FN fix: reduce with call arg matching block param — not suppressed by RuboCop
def apply_filters(content, filters)
  filters.reduce(content) { |content, filter| filter.apply(content) }
                             ^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `content`.
end

# FN fix: File.open with call arg matching block param
def overwrite_file(file, new_content)
  File.open(file, "w") { |file| file.puts new_content }
                          ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `file`.
end

# FN fix: inject with call arg matching block param (corpus: elasticsearch-ruby)
def execute(client, test = nil)
  @definition.each.inject(client) do |client, (method_chain, args)|
                                      ^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `client`.
    chain = method_chain.split('.')
    client
  end
end

# FN fix: each_with_object with arg matching block param (corpus: locomotivecms)
def extract_exposures(exposures, hash = {}, prefix = nil)
  exposures.each_with_object(hash) do |exposure, hash|
                                                 ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `hash`.
    key = "#{prefix}#{exposure}"
    hash[key.to_sym] = exposure
  end
end

# FN fix: Dir.chdir with arg matching block param (corpus: foreman)
def mkchdir(dir)
  FileUtils.mkdir_p(dir)
  Dir.chdir(dir) do |dir|
                     ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `dir`.
    yield(File.expand_path(dir))
  end
end

# FN fix: Find.find with arg matching block param (corpus: fpm)
def remove_compiled_files(path)
  Find.find(path) do |path|
                      ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `path`.
    FileUtils.rm(path) if path.end_with?('.pyc')
  end
end

# FN fix: custom method with arg matching block param (corpus: ransack)
def initialize(reflection, children, polymorphic_class = nil)
  swapping_reflection_klass(reflection, polymorphic_class) do |reflection|
                                                               ^^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `reflection`.
    super(reflection, children)
  end
end

# FN fix: Zip::File.open with arg matching block param (corpus: oxml_xxe)
def read_rels(zipfile, fil_r)
  Zip::File.open(zipfile) do |zipfile|
                              ^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `zipfile`.
    zipfile.read(fil_r)
  end
end

# FN fix: lambda param shadowed by reduce block param (corpus: moneta)
def make_encoder(transforms)
  lambda do |value|
    transforms.reduce(value) do |value, transform|
                                 ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `value`.
      transform.encode(value)
    end
  end
end

# FN fix: method param shadowed via with_connection block (corpus: ruby-polars)
def write_database(connection, table_name, if_table_exists)
  with_connection(connection) do |connection|
                                  ^^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `connection`.
    connection.table_exists?(table_name)
  end
end

# FN fix: condition-assigned local, block in multi-statement then-body
def switch_items(all_buildable_items, pod_names_to_switch, pod_name)
  if pod = (all_buildable_items.detect { |t| t.name == pod_name } || all_buildable_items.detect { |t| t.root_name == pod_name })
    dependencies = []
    all_buildable_items.each do |pod|
                                 ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `pod`.
      if !(pod.dependency_names & pod_names_to_switch).empty?
        dependencies.push(pod.root_name)
      end
    end
    pod_names_to_switch += dependencies
  end
end

# FN fix: variable in elsif body, block nested in multi-statement else block
def serialize_dao(file_versions_to_display, digital_object, xml, content, fragments)
  if file_versions_to_display.empty?
    xml.dao({})
  elsif file_versions_to_display.length == 1
    file_version = file_versions_to_display.first
    xml.dao(file_version) {}
  else
    xml.daogrp({}) {
      xml.daodesc { sanitize_mixed_content(content, xml, fragments, true) } if content
      file_versions_to_display.each do |file_version|
                                        ^^^^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `file_version`.
        xml.daoloc(file_version)
      end
    }
  end
end

# FN fix: condition-assigned local, block in multi-statement body
def build_non_att(non_att_children)
  if (nac = non_att_children).any?
    handle(nac)
    non_att_children.each { |nac| duplicate(nac) }
                             ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `nac`.
  end
end

# FN fix: condition-assigned local, block nested in expression
def decorate_output(stdout)
  ret = +""
  if @output_stdout && (s = stdout.read) != ""
    ret << s.inject("") { |s, line| s + "# >> #{line}".chomp + "\n" }
                           ^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `s`.
  end
  ret
end

# FN fix: outer var from if-branch, block nested in multi-statement Proc body
def build_hook(block)
  if block
    hook_name = :before
    options = {}
    hook = { block: block, options: options, name: hook_name }
    use(hook)
  else
    Proc.new {
      filtered_hooks = hooks.reject { |hook| hook[:options] }
                                       ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `hook`.
      filtered_hooks
    }
  end
end

# FN fix: outer var from multi-statement if-branch, inner block in multi-statement else block body
def get_login_info(accounts, uri)
  username, password = nil, nil
  unless accounts.empty? || force_new
    if force_account
      host, username, password = accounts.find { |h, u, p| force_account == "#{u}@#{h}" }
      unless username && password
        say "No previous account"
      end
    else
      choose do |menu|
        accounts.each do |host, olduser, oldpw|
                          ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `host`.
          menu.choice("Use the account info for #{olduser}@#{host}") { username, password = olduser, oldpw }
        end
        menu.choice("Use a new account") { }
        menu.prompt = "Account selection? "
      end
    end
  end
end

# FN fix: operator write exposes local before RHS block runs
def total_sum_at_index(index)
  total ||= (0..@number_of_plots - 1).inject(0) { |total, i| total + data[i][index] }
                                                   ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `total`.
  total
end

# FN fix: for loop collection block param shadows pre-existing local
def process_yaku_stats(yaku_stats)
  yaku = nil
  for yaku, count in yaku_stats.sort_by{|yaku, count| -count}
                                         ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `yaku`.
    puts yaku
  end
end

# FN fix: for loop collection block param shadows pre-existing local (count)
def process_dora_stats(dora_stats)
  count = nil
  for dora, count in dora_stats.sort_by{|dora, count| -count}
                                               ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `count`.
    puts count
  end
end

# FN fix: method param shadowed by block param in call with same-name arg
def complete_text(text, pos)
  text.modify_for_completion(text, pos) do |string, trigger, pos|
                                                             ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `pos`.
  end
end

# FN fix: method param shadowed by block param in simple iteration
# (corpus: appoxy/aws require_relative.rb:10)
def require_relative(path)
  desired_path = File.expand_path(path)
  shortest = desired_path
  $:.each do |path|
              ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `path`.
    shortest = path if path.size < shortest.size
  end
  require shortest
end

# FN fix: local var shadowed by block param in method-call block
# (corpus: braintree spec_helper.rb:182)
class SpecHelper
  def self.simulate_form_post(url)
    http = Net::HTTP.new("localhost", 80)
    http.use_ssl = true
    http.start do |http|
                   ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `http`.
      request = Net::HTTP::Post.new(url)
      http.request(request)
    end
  end
end

# FN fix: case/when — variable in single-stmt when body, block in different single-stmt when body
# block_cond_parent_suppresses incorrectly propagates through single-stmt chains
# (corpus: AuthorizeNet/sdk-ruby xml_response.rb:140)
def handle_node_type(node_desc, xml, node_name)
  case node_desc[node_name]
  when Symbol
    node = xml.at_css(node_name.to_s)
  when EntityDescription
    if node_desc[:multivalue].nil?
      entity = build_entity(xml)
    else
      xml.css(node_name.to_s).each do |node|
                                       ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `node`.
        build_entity(node)
      end
    end
  end
end

# FN fix: variable at method scope, block inside if branch
# (corpus: Restream/redmine_elasticsearch parent_project.rb:76)
def allowed_to_query(user, permission)
  role = user.logged? ? non_member : anonymous
  if role.allowed_to?(permission)
    use(role)
  end
  if user.logged?
    user.projects_by_role.each do |role, projects|
                                   ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `role`.
      use(role, projects)
    end
  end
end

# FN fix: def inside conditional — the conditional branch context should not
# leak through the def scope boundary. Block params inside the def that shadow
# method params should still be flagged.
unless Kernel.respond_to?(:require_relative)
  def require_relative(path)
    $:.each do |path|
                ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `path`.
      path += '/'
    end
  end
end

# Same pattern with if
if RUBY_VERSION < '2.0'
  def my_method(data)
    data.map do |data|
                 ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `data`.
      data.to_s
    end
  end
end

# def self.foo inside conditional
if defined?(Net::HTTP)
  def self.simulate(http)
    http.start do |http|
                   ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `http`.
      http.request
    end
  end
end

# FN fix: nested else branch retains outer local from enclosing else scope
# (corpus: cenit-io/cenit parser.rb:396)
def do_parse_json(json_schema, data_type, options)
  if json_schema["type"] == "complex"
    skip
  else
    property_schema = nil
    if (properties = json_schema["properties"])
      if properties.size == 1
        property_schema = data_type.merge_schema(properties.values.first)
      else
        properties.each do |property_name, property_schema|
                                           ^^^^^^^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `property_schema`.
          next if options[:ignore].include?(property_name.to_sym)
          property_schema = data_type.merge_schema(property_schema)
        end
      end
    end
  end
end

# FN fix: reassignment RHS block param still shadows previously-declared local
# (corpus: dbrady/ssh-config config_file.rb:139)
def unalias!(*args)
  if args.size >= 2
    name = args.shift
    section = @sections_by_name[name]
  else
    section = @sections.select { |section| section.has_alias?(args[0]) }.first
                                  ^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `section`.
  end
  section
end

# FN fix: proc param in branch-local hash value shadows earlier branch local
# (corpus: ruby/tk fontchooser.rb:167)
def set_for(target)
  if target.kind_of?(TkFont)
    configs = {
      font: target.actual_hash
    }
  elsif target.kind_of?(Hash)
    fnt = target[:font] rescue ""
    fnt = fnt.actual_hash if fnt.kind_of?(TkFont)
    configs = {
      font: fnt,
      command: proc { |fnt, *args|
                       ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `fnt`.
        target[:font] = TkFont.actual_hash(fnt)
      }
    }
  else
    configs = {}
  end
  configs
end

# FN fix: loop counter remains visible after while before nested each block
# (corpus: ruby/typeprof box.rb:663)
def pass_arguments(a_args, opt_positionals, rest_positionals)
  start_rest = 0
  end_rest = a_args.positionals.size
  i = 0
  while i < opt_positionals.size && start_rest < end_rest
    i += 1
    start_rest += 1
  end

  if start_rest < end_rest
    if rest_positionals
      (start_rest..end_rest - 1).each do |i|
                                          ^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `i`.
        use(i)
      end
    end
  end
end

# FN fix: multi-statement when body should not inherit outer else-branch suppression
def handle_grouped_chunk(grouped_chunk)
  grouped_chunk.each do |grp|
    if grp.count == 1
      line = grp[0]
      parse(line)
    else
      case
      when grp[0] =~ /Class:/
        grp = grp.map(&:strip)
        vals = []
        grp.each do |line|
                     ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `line`.
          vals << line
        end
      end
    end
  end
end

# FN fix: nested if body inside else branch should still see earlier branch local
def all(index, direction, ids_key, limit)
  if primary?
    use_primary
  else
    if index && index.options[:unique]
      id = fetch(prepared_index)
      find(id)
    else
      if direction.to_s == "desc"
        ids = fetch_ids(ids_key, limit).compact
        ids.collect { |id| find(id) }
                       ^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `id`.
      end
    end
  end
end

# FN fix: earlier for-loop index remains visible inside later collection block
def print_yaku_stats(raw_actions, name_to_yaku_stats)
  for raw_action in raw_actions
    if raw_action.type == :hora
      for yaku, fan in raw_action.yakus
        use(yaku, fan)
      end
    end
  end

  for name, yaku_stats in name_to_yaku_stats.sort
    for yaku, count in yaku_stats.sort_by { |yaku, count| -count }
                                             ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `yaku`.
    end
  end
end

# FN fix: later for-loop collection block shadows prior for-loop index
def print_dora_stats(name_to_yaku_stats, name_to_dora_stats)
  for name, yaku_stats in name_to_yaku_stats.sort
    for yaku, count in yaku_stats.sort_by { |yaku, count| -count }
      use(yaku, count)
    end
  end

  for name, dora_stats in name_to_dora_stats.sort
    for dora, count in dora_stats.sort_by { |dora, count| -count }
                                                   ^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `count`.
    end
  end
end

# FN fix: proc hash value in else branch still shadows sibling-branch local
def set_for_else_branch(target)
  if target.kind_of?(TkFont)
    configs = { font: target.actual_hash }
  elsif target.kind_of?(Hash)
    fnt = target[:font] rescue ""
    fnt = fnt.actual_hash if fnt.kind_of?(TkFont)
    configs = { font: fnt }
  else
    configs = {
      font: target.cget_tkstring(:font),
      command: proc { |fnt, *args|
                       ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `fnt`.
        target.font = normalize(fnt, target)
      }
    }
  end
  configs
end

# FN fix: rightward pattern matching locals are outer scope for later blocks
def run_completion(text)
  @params => { position: pos }
  text.modify_for_completion(text, pos) do |string, trigger, pos|
                                                             ^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `pos`.
  end
end

# FN fix: same-branch unless body still flags direct child block params
def generate_checks(chat)
  @test_paths.each do |status_path|
    unless @checked_paths[status_path]
      check_list = []
      test = chat.copy_request
      test.set_dir(status_path)
      check_list.each do |test|
                          ^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `test`.
        run(test)
      end
    end
  end
end

# FN fix: explicit begin/rescue wrapper does not make the block a direct if child
def calc_error(gcps)
  if gcps.size > 3
    begin
      dest_set = gcps.map(&:dest_coords)
      source_set = gcps.map(&:source_coords)
      cx_dst = cy_dst = cx_src = cy_src = 0

      x = Matrix[* dest_set.map { |dst| [dst[0] - cx_dst] }].transpose
      y = Matrix[* dest_set.map { |dst| [dst[1] - cy_dst] }].transpose
      aa = Matrix[* source_set.map { |src| [1.0, src[0] - cx_src, src[1] - cy_src] }]
      q = (aa.transpose * aa).inverse
      a = q * (aa.transpose * x.transpose)
      b = q * (aa.transpose * y.transpose)
      w = [a[1, 0], b[1, 0], a[2, 0], b[2, 0], cx_dst, cy_dst]

      source_set.each do |x, y|
                          ^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `x`.
                             ^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `y`.
        use(w, x, y)
      end
    rescue StandardError
      handle_error
    end
  end
end

# FN fix: case/when single-statement branch should still flag nested shadowing
def deploy_linux(protocol, target, context)
  case protocol
  when "ssh"
    self.withConnection(target) do |connection|
      self.class.withConnection(connection, context) do |connection|
                                                         ^^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `connection`.
        use(connection)
      end
    end
  end
end

# FN fix: if single-statement branch should still flag nested shadowing
def backup_linux(target, context)
  if target
    self.withConnection(target) do |connection|
      self.class.withConnection(connection, context) do |connection|
                                                         ^^^^^^^^^^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `connection`.
        use(connection)
      end
    end
  end
end

# FN fix: nested receiver call in conditional branch still shadows outer block param
def yomiage_process(last)
  if last.tag_bundle
    last.tag_bundle.each do |e|
      e.each do |e|
                 ^ Lint/ShadowingOuterLocalVariable: Shadowing outer local variable - `e`.
        talk(e.name)
      end
    end
  end
end
