def some_method
  some_var = 1
  do_something(some_var)
end

def other_method
  _unused = 1
  do_something
end

# Compound assignment += reads the variable, so the initial assignment is used
def compound_plus_equals
  count = 0
  3.times { count += 1 }
end

# Compound assignment in block
def compound_in_block
  rating = 1
  items.each { |item| item.update!(rating: rating += 1) }
end

# Or-assignment ||= reads the variable first
def or_assign
  hash_config = nil
  stub(:db_config, -> { hash_config ||= build_config }) { run }
end

# And-assignment &&= reads the variable first
def and_assign
  value = true
  value &&= check_condition
  do_something(value)
end

# Singleton method definition uses variable as receiver
def singleton_method_on_local
  conn = get_connection
  def conn.requires_reloading?
    true
  end
  pool.clear_reloadable_connections
end

# Another singleton method pattern
def define_method_on_object
  time = @twz.time
  def time.foo; "bar"; end
  @twz.foo
end

# Bare super implicitly forwards all method parameters
def self.instantiate_instance_of(klass, attributes, column_types = {}, &block)
  klass = superclass
  super
end

# String concatenation compound assignment
def compound_string_concat
  lines = "HEY\n" * 12
  assert_no_changes "lines" do
    lines += "HEY ALSO\n"
  end
end

# Variable assigned in block but read in nested block
describe "something" do
  it "does something" do
    app = create(:app)
    problem = create(:problem, app: app)
    expect do
      destroy(problem.id)
    end.to change(Problem, :count).by(-1)
  end
end

# Variable read inside same block (not nested)
items.each do |item|
  x = compute(item)
  process(x)
end

# Bare `binding` captures all local variables, so assignments are not useless
def render_template
  github_user = `git config github.user`.chomp
  template = File.read("template.erb")
  ERB.new(template).result(binding)
end

# `binding` in a block also captures all locals in that scope
task :announce do
  version = ENV["VERSION"]
  github_user = `git config github.user`.chomp
  puts ERB.new(template).result(binding)
end

# Variable assigned in block, read after block in outer scope (blocks share
# enclosing scope in Ruby for variables declared in the outer scope)
describe "block with outer read" do
  result = nil
  [1, 2, 3].each { |x| result = x * 2 }
  puts result
end

# Variable used across nested blocks (not siblings)
describe "nested blocks" do
  it "works" do
    token = create(:token)
    3.times do
      validate(token)
    end
  end
end

# All sibling blocks use their own token (each is used)
describe "all siblings used" do
  it "first" do
    token = create(:token)
    expect(token).to be_valid
  end
  it "second" do
    token = create(:token)
    expect(token).to be_present
  end
end

# `binding` in a nested block captures locals from the outer block scope
describe "binding in nested block" do
  version = "1.0"
  channel = "stable"
  items.each { puts ERB.new(tmpl).result(binding) }
end

# Variable assigned in block and read in sibling block's descendant (via
# ancestor scope) — this is NOT a sibling read, the outer describe scope
# sees the read.
describe "ancestor read" do
  total = 0
  items.each { |x| total += x }
  it "checks total" do
    expect(total).to eq(42)
  end
end

# Variable initialized to nil, reassigned inside a lambda, read after block.
# Common in Rails test stubs — the lambda captures the outer variable.
describe "lambda capture reassignment" do
  it "captures display image" do
    display_image_actual = nil
    stub :show, ->(img) { display_image_actual = img } do
      take_screenshot
    end
    assert_match(/screenshot/, display_image_actual)
  end
end

# Multiple variables captured by lambdas at different nesting levels
describe "multi-level lambda capture" do
  it "captures at different levels" do
    captured_a = nil
    captured_b = false
    stub :foo, ->(x) { captured_a = x } do
      stub :bar, -> { captured_b = true } do
        run_action
      end
    end
    assert captured_b
    assert_match(/expected/, captured_a)
  end
end

# RSpec `.change { var }` matcher — the block reads the variable
describe "change matcher reads variable" do
  it "tracks changes" do
    count = 0
    items.each { count += 1 }
    expect { do_something }.to change { count }
  end
end

# Variable assigned in parent block, written+read across multiple siblings
# (the "error = nil" Rails pattern)
describe "shared variable across siblings" do
  error = nil
  it "assigns error" do
    error = validate(input)
  end
  it "checks error" do
    assert_nil error
  end
end

# Accumulator pattern — array initialized in parent scope, appended in block,
# read in sibling block (common in Rails test setup)
describe "accumulator across siblings" do
  sponsors = []
  users.each { |u| sponsors << u if u.sponsor? }
  it "has sponsors" do
    expect(sponsors).not_to be_empty
  end
end

# Three-level nesting: describe > context > it, variable in describe read in it
describe "deep nesting" do
  shared_val = compute_value
  context "when enabled" do
    it "uses shared_val" do
      expect(shared_val).to eq(42)
    end
  end
end

# Reassigned in single-branch if, referenced after branching
def reassign_in_branch(flag)
  foo = 1
  if flag
    foo = 2
  end
  foo
end

# Assigned in each branch and referenced after
def assign_both_branches(flag)
  if flag
    foo = 2
  else
    foo = 3
  end
  foo
end

# Variable reassigned at end of loop body, referenced in next iteration
def loop_reassign
  total = 0
  foo = 0
  while total < 100
    total += foo
    foo += 1
  end
  total
end

# Variable referenced in loop condition
def loop_condition_ref
  foo = 0
  while foo < 100
    foo += 1
  end
end

# Assignment in if branch referenced in another if branch
def cross_branch_ref(flag_a, flag_b)
  if flag_a
    foo = 1
  end
  if flag_b
    puts foo
  end
end

# Reassigned in a block (block may not execute)
def reassign_in_block
  foo = 1
  puts foo
  1.times do
    foo = 2
  end
end

# Variable assigned in branch and referenced after
def branch_then_read(flag)
  foo = 1
  if flag
    foo = 2
  end
  foo
end

# For loop variable that IS referenced
for item in items
  do_something(item)
end

# Variable assigned in modifier condition and read
def modifier_condition
  a = nil
  puts a if (a = 123)
end

# Variable used in loop condition (while)
def while_condition
  line = gets
  while line
    process(line)
    line = gets
  end
end

# Unreferenced variable reassigned in block (block may run multiple times)
def const_name(node)
  const_names = []
  const_node = node
  loop do
    namespace_node, name = *const_node
    const_names << name
    break unless namespace_node
    break if namespace_node.type == :cbase
    const_node = namespace_node
  end
  const_names.reverse.join('::')
end

# Variable reassigned in a loop body, used in next iteration
def reassign_in_while
  ret = 1
  param = 0
  while param < 40
    param += 2
    ret = param + 1
  end
  ret
end

# Assigning in branch with block
def assign_in_branch_with_block
  changed = false
  if Random.rand > 1
    changed = true
  end
  [].each do
    changed = true
  end
  puts changed
end

# Variable initialized before begin/rescue, reassigned inside, read after
# The initial assignment is NOT useless: if an exception fires before the
# reassignment, the initial value is what remains.
def begin_rescue_init
  result = nil
  begin
    result = do_something
  rescue => e
    handle_error(e)
  end
  result
end

# Variable initialized before begin/rescue, rescue re-raises
# RuboCop still does not flag the initial assignment because the begin body
# might partially execute before reaching the reassignment.
def begin_rescue_reraise
  result = nil
  begin
    driver = create_driver
    result = driver.process(options)
    save!
  rescue => e
    message = handle_error(e)
    save!
    raise e, message
  end
  result
end

# Variable initialized before begin with multiple rescues
def begin_multiple_rescue
  data = {}
  begin
    data = fetch_data(url)
  rescue Timeout::Error
    log_timeout
  rescue => e
    log_error(e)
  end
  data
end

# Singleton class reads the variable (class << obj)
def singleton_class_receiver
  obj = Object.new
  class << obj
    def foo; "bar"; end
  end
end

# Singleton class with method calls after
def singleton_class_with_method
  clone_obj = original.clone
  class << clone_obj
    CLONE_CONST = :clone
  end
end

# Variable assigned before begin/ensure (no rescue) — not useless
# The begin body might raise, so `result` remains nil and ensure runs.
def begin_ensure_init
  result = nil
  begin
    result = do_something
  ensure
    cleanup(result)
  end
end

# Variable assigned before begin, used in both success and rescue paths
def begin_rescue_used_both_paths
  data = default_data
  begin
    data = fetch_data(url)
  rescue => e
    log_error(data, e)
  end
  process(data)
end

# Retry counter incremented in begin body, checked in rescue
def retry_with_counter
  counter = 0
  begin
    counter += 1
    perform_work
  rescue
    retry if counter < 3
  end
end

# Boolean flag set in begin body, read in rescue
def flag_set_in_begin_body
  success = false
  begin
    do_work
    success = true
  rescue => e
    log_failure(e) unless success
  end
end

# Timestamp captured in begin body, read in rescue
def timing_in_begin_rescue
  started = Time.now
  begin
    started = Time.now
    slow_operation
  rescue => e
    elapsed = Time.now - started
    report_timeout(e, elapsed)
  end
end

# Begin body and rescue body both write the same variable — not useless
# because begin and rescue are alternative paths (only one executes)
def commit(action, params)
  begin
    raw = ssl_post(action, params)
    response = parse(raw)
  rescue
    raw = fallback_response
    response = parse(raw)
  end
  response
end

# Same pattern with ensure
def fetch_with_retry
  begin
    result = try_fetch
  rescue
    result = default_value
  end
  result
end

# Pattern matching captures are not flagged (RuboCop does not flag them)
def pattern_match_unused_capture
  case [1, 2, 3]
  in [_, middle, *rest]
    puts middle
  end
end

# Pattern matching capture with named pin
def pattern_match_named_capture
  case expr
  in [Integer => x, Integer => y]
    puts x
  end
end

# Pattern matching with hash patterns
def pattern_match_hash
  case data
  in { name: name, **rest }
    puts name
  end
end

# Rightward pattern matching captures are not flagged either
def rightward_pattern_match_array
  [1, 2, 3] => [first, *rest]
end

# Rightward hash-pattern capture without later reads still matches RuboCop
def rightward_pattern_match_hash
  case
  when true
  end => { foo: }
end

# Multiple rescue clauses are mutually exclusive. A later read after the
# rescue chain uses whichever branch assigned the value.
def rescue_chain_value
  begin
    work
  rescue SomeError
    score = 0
  rescue OtherError
    score = 99
  end
  puts score
end

# FP fix: assignment in || condition — short-circuit means only one branch executes
def assign_in_or_condition(item, request)
  if item.respond_to?(method = "#{request}_path") || item.respond_to?(method = "path")
    item.send(method)
  end
end

# FP fix: assignment in || condition — variable used after
def assign_in_or_with_key(expression, fields)
  fields.each do |field|
    if expression.key?(key = field.to_s) || expression.key?(key = field.to_sym)
      expression.delete(key)
    end
  end
end

# FP fix: assignment in || condition — XPath fallback pattern
def assign_in_or_xpath(xml)
  if (root = search(xml, '//Response')) || (root = search(xml, '//AltResponse'))
    root.elements.to_a
  end
end

# FP fix: begin/end until (do-while) loop — assignment in body feeds the condition
def begin_end_until_loop(flatfile)
  begin
    line = flatfile.gets
  end until line.nil?
end

# FP fix: begin/end while (do-while) loop — same pattern
def begin_end_while_loop(socket, endpoint)
  begin
    rc = socket.connect(endpoint)
  end while rc == -1
end

# FP fix: inline rescue modifier — assignment in rescue path, read later
def inline_rescue_assign
  params = ["a", "1.0", "2.0", "3.0", "4.0"]
  err = nil
  red_low = Float(params[1]) rescue err = "red low"
  yellow_low = Float(params[2]) rescue err = "yellow low"
  yellow_high = Float(params[3]) rescue err = "yellow high"
  red_high = Float(params[4]) rescue err = "red high"
  raise "Invalid #{err}" if err
  [red_low, yellow_low, yellow_high, red_high]
end

# FP fix: assignment in implicit-rescue method body, variable read in rescue handler.
# If `extract_code` raises, the rescue handler reads `code` (via `code:` shorthand)
# with the value from the first assignment. Not useless.
def get_embedded_sql_answer(text)
  code = text[/^SQLQuery: (.*)/, 1]
  code = extract_code(text)
  Boxcars.debug code, :yellow
  output = clean_up_output(code)
  Result.new(status: :ok, answer: output, code: code)
rescue StandardError => e
  Result.new(status: :error, answer: nil, explanation: e.message, code: code)
end

# FP fix: assignment before ensure block, variable read in ensure.
# If `start(...)` raises, the ensure block reads `obj` with the initial value.
def call_target(target_num)
  obj = {}
  obj = start(target_num: target_num)
  process(obj)
ensure
  cleanup(obj)
end
