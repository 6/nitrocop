def some_method
  foo = 1
  puts foo
  1.times do |bar|
  end
end
def some_method
  foo = 1
  puts foo
  1.times do
    foo = 2
  end
end
def some_method
  _ = 1
  puts _
  1.times do |_|
  end
end
def some_method
  _foo = 1
  puts _foo
  1.times do |_foo|
  end
end

# Variables from sibling blocks should not be treated as outer locals
def sibling_blocks
  [1].each { |x| y = x + 1; puts y }
  [2].each { |y| puts y }
end

# Variables from sibling lambdas should not leak
def sibling_lambdas
  a = lambda { |n| n = n.to_s; puts n }
  b = lambda { |n| puts n }
  a.call(1)
  b.call(2)
end

# Variables defined inside a block should not shadow in sibling block
class MyClass
  scope :secured, ->(guardian) { ids = guardian.secure_ids; puts ids }
  scope :with_parents, ->(ids) { where(ids) }
end

# Nested block variables should not leak to outer scope
def nested_blocks
  items.each do |item|
    item.children.each { |child| value = child.name; puts value }
  end
  other_items.each do |other|
    other.parts.each { |value| puts value }
  end
end

# Different branches of case/when - not flagged
def different_branches
  case filter
  when "likes-min"
    value = values.last
    value if value =~ /\A\d+\z/
  when "order"
    values.flat_map { |value| value.split(",") }
  end
end

# Variable used in declaration of outer — block is the RHS of the assignment
def some_method
  foo = bar { |foo| baz(foo) }
end

# Variable used in return value assignment of if
def some_method
  foo = if condition
          bar { |foo| baz(foo) }
        end
end

# Different branches of if condition
def some_method
  if condition?
    foo = 1
  elsif other_condition?
    bar.each do |foo|
    end
  else
    bar.each do |foo|
    end
  end
end

# Different branches of unless condition
def some_method
  unless condition?
    foo = 1
  else
    bar.each do |foo|
    end
  end
end

# Different branches of if condition in a nested node
def some_method
  if condition?
    foo = 1
  else
    bar = [1, 2, 3]
    bar.each do |foo|
    end
  end
end

# Different branches of case condition
def some_method
  case condition
  when foo then
    foo = 1
  else
    bar.each do |foo|
    end
  end
end

# Sibling block variables (from prior block body) don't shadow
def x(array)
  array.each { |foo|
    bar = foo
  }.each { |bar|
  }
end

# Class-level begin block vars don't shadow method-level block params
class MyTranslator
  MAPPING =
    begin
      from = "abc"
      to = "xyz"
      from.chars.zip(to.chars)
    end

  def translate(name)
    MAPPING.each { |from, to| name.gsub!(from, to) }
    name
  end
end

# Later method params are not visible in earlier default lambdas
def build_handlers(
  outer: ->(cursor) { cursor },
  inner: ->(item, cursor) { [item, cursor] },
  cursor: nil
)
  [outer, inner, cursor]
end

# Top-level locals do not leak into class body proc scopes
command = "outer"

class Worker
  HANDLER = proc do |command|
    puts command
  end
end

# Ractor.new block — shadowing is intentional (Ractor can't access outer scope)
def start_ractor(*args)
  Ractor.new(*args) do |*args|
    puts args.inspect
  end
end

# Ractor.new with single param
def start_worker(p)
  Ractor.new(p) do |p|
    puts p.inspect
  end
end

# Variable assigned in when condition, block param in when body
def process(env)
  case
  when decl = env.fetch(:type, nil)
    decl.each do |decl|
      puts decl
    end
  when decl = env.fetch(:other, nil)
    decl.map do |decl|
      decl.to_s
    end
  end
end

# FP fix: variable assigned in elsif condition, block in different elsif body
def parse_input(params)
  if msgpack = params['msgpack']
    parse_msgpack(msgpack)
  elsif js = params['json']
    parse_json(js)
  elsif ndjson = params['ndjson']
    ndjson.split(/\r?\n/).each do |js|
      parse_json(js)
    end
  end
end

# FP fix: variable assigned in if condition, block in else branch
def find_account(email)
  if a = lookup(email)
    a
  else
    regexen.argfind { |re, a| re =~ email && a }
  end
end

# FP fix: variable assigned in case predicate, block in when body
def format_value(key, opts)
  case value = send(key)
  when String then "#{opt_key(key, opts)}=#{value.inspect}"
  when Array  then value.map { |value| "#{opt_key(key, opts)}=#{value.inspect}" }
  else opt_key(key, opts)
  end
end


# FP fix: variable assigned in one when body, block param in different when body
def process_slug(slug)
  case slug
  when 'items'
    node = find_node('#Items')
    node.css('li').each { |n| register(n) }
  when 'utils'
    css('dl > dt').each do |node|
      register(node)
    end
  end
end

# FP fix: 3 when branches - block param in one, local var in another, block param in third
def process_data(slug)
  case slug
  when 'first'
    css('h2').each do |heading|
      heading.css('a').each do |node|
        register(node)
      end
    end
  when 'second'
    node = find('#Section')
    node = node.next while node.name != 'ul'
    node.css('li').each do |n|
      register(n)
    end
  when 'third'
    css('dl > dt').each do |node|
      register(node)
    end
  end
end

# FP fix: variable in elsif body, block param in next elsif condition
def unwrap(node)
  if condition_a?(node)
    do_a(node)
  elsif condition_b?(node)
    *before, list = node.children
    [*before, transform(list)]
  elsif items.any? { |list| list === node }
    do_c(node)
  else
    node
  end
end

# FP fix: variable assigned in if-condition, block in then-body (tap pattern)
def find_or_create_item(page)
  record = if item = page.menu_item
             item.tap { |item| item.parent_id = nil }
           else
             build_item(page)
           end
  record
end

# FP fix: variable in if-branch, block nested in another block in else-branch
# (e.g., active-hash pluck pattern: column_name in if, column_names.map { |column_name| } in else)
def pluck(*column_names)
  if column_names.length == 1
    column_name = column_names.first
    all.map { |r| r.send(column_name) }
  else
    all.map { |r| column_names.map { |column_name| r.send(column_name) } }
  end
end

# FP fix: same pattern — variable in if-branch, block in else via map (neo4j/activegraph)
def pluck_results(columns, result)
  if columns.size == 1
    column = columns[0]
    result.map { |row| row[column] }
  else
    result.map { |row| columns.map { |column| row[column] } }
  end
end


# FP fix: assignment RHS block in conditional branch body (elsif)
def self.aws_image_id
  if self[:aws][:aws_image_id]
    return self[:aws][:aws_image_id]
  elsif (self[:aws][:ubuntu_release] && self[:aws][:region])
    ami = Ubuntu.release(self[:aws][:ubuntu_release]).amis.find do |ami|
      ami.arch == "i386" && ami.region == self[:aws][:region]
    end
    return ami.name if ami
  end
end

# FP fix: assignment RHS block in else branch
def build_cmd_output_string(cmd_params, packet)
  if cmd_params.nil? or cmd_params.empty?
    output_string = "simple"
  else
    params = []
    cmd_params.each do |key, value|
      item = packet['items'].find { |item| item['name'] == key.to_s }
      params << item
    end
  end
end

# FP fix: assignment RHS block in if branch
def find(id)
  if loaded? || proxy_owner.new_record?
    record = proxy_target.find { |record| record.id == id }
  elsif find_with_proc?
    record = other_find(id)
  end
end

# FP fix: assignment RHS block in while loop
def main(host)
  add_host(host, nil)
  while @hosts.select {|h| h[:processed]}.size < 10
    h = @hosts.select {|h| !h[:processed] }[0]
    get_links h
    h[:processed] = true
  end
end

# FP fix: defs method params should not shadow outer block params
def setup_stubs
  MassiveRecord::Wrapper::Table.stub!(:new) do |*args|
    table = new_table_method.call(*args)
    def table.find(*args)
      args
    end
    def table.all(*args)
      [args]
    end
    table
  end
end

# FP fix: def self.method params inside block scope
class MyModel
  class_eval do |attributes|
    def self.insert(attributes, **options)
      super(map_attributes(attributes), **options)
    end
    def self.insert!(attributes, **options)
      super(map_attributes(attributes), **options)
    end
  end
end

# FP fix: block params inside def self body should not see outer block params
class MyMapper
  SOURCES.each do |attributes|
    class_eval do
      def self.lockbox_map_attributes(records)
        records.map do |attributes|
          attributes
        end
      end
    end
  end
end

# FP fix: block params inside singleton class body should not see outer block params
items.each do |char|
  class << self
    methods_postfixes.each do |char|
      char
    end
  end
end

# Corrected: block nested inside another block in else-branch of if.
# RuboCop's variable_node(variable) returns variable.scope.node.parent
# which is the choose block. The choose block IS if.else_branch, so
# same_conditions_node_different_branch? returns true → suppressed.
# Previously incorrectly classified as an offense.
def get_login_info(sources)
  username, password = nil, nil
  unless sources.empty?
    if force_account
      host, username, password = sources.find { |h, u, p| h == target }
    else
      choose do |menu|
        sources.each do |host, olduser, oldpw|
          menu.choice(olduser, host)
        end
      end
    end
  end
end

# Thread.new(value) in else branch — RuboCop suppresses via check 6:
# variable_node (begin wrapping else body) == if.else_branch
def threaded_or_sequential(lib, &block)
  if use_threads?
    Thread.new { block.call(lib) }
  else
    value = block.call(lib)
    Thread.new(value) { |value| value }
  end
end

# Variable in if-branch, multi-statement block in else-branch of same if
# Suppressed by RuboCop check 6: variable_node == else_branch
def process_with_fallback(items, flag)
  if flag
    val = compute(items)
    use(val)
  else
    log = setup_log
    items.each do |val|
      val.process
      log.info(val)
    end
  end
end

# Variable and block both in same multi-statement else branch
# Suppressed by RuboCop check 6: variable_node == else_branch
def handle_result(items, flag)
  if flag
    default_value
  else
    val = compute(items)
    items.each { |val| process_val(val) }
  end
end

# FP fix: reassignment with block param — RuboCop suppresses via structural
# equality of lvasgn nodes in `variable_used_in_declaration_of_outer?`.
# When `var = expr { |var| }` is repeated, the lvasgn nodes are structurally
# equal, so the ancestor check matches.
def test_method
  graph = RDF::Graph.new do |graph|
    graph << reader
  end
  graph.statements

  graph = RDF::Graph.new do |graph|
    graph << reader
  end
  graph.statements
end

# FP fix: reassignment with block param (orthoses pattern — multiple unless blocks between)
def test_lazy(t)
  called = []
  tp = Orthoses::LazyTracePoint.new(:call) do |tp|
    called << tp
  end
  tp.enable {}

  unless !tp.enabled?
    t.error("nope")
  end

  called = []
  tp = Orthoses::LazyTracePoint.new(:call) do |tp|
    called << tp
  end
  tp.enable {}
end

# FP fix: reassignment with detect block param
def close_window(title)
  if title
    win = Redcar.app.windows.detect{|win| win.title == title }
  else
    win = Redcar.app.focussed_window
  end
  win.close
end

# FP fix: reassignment with execute block param
def check_package(name, opts)
  result = execute("zypper se #{name}", opts) { |result| result }
  result.stdout.include?('found')
end

# FP fix: variable in if-branch, block deep in else-branch nested in case/when
# (corpus: blue_hydra parser.rb:284 — `line` in if, `grp.map do |line|` in else/case/when)
def handle_grouped_chunk(grouped_chunk)
  grouped_chunk.each do |grp|
    if grp.count == 1
      line = grp[0]
      parse(line)
    else
      case
      when grp[0] =~ /Class:/
        grp.each do |line|
          process(line)
        end
      when grp[0] =~ /Manufacturer/
        grp.map do |line|
          format(line)
        end
      end
    end
  end
end
