def some_method
  some_var = 1
  ^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `some_var`.
  do_something
end

def other_method
  x = compute_value
  ^ Lint/UselessAssignment: Useless assignment to variable - `x`.
  y = another_value
  do_something(y)
end

def third_method
  unused = 'hello'
  ^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `unused`.
end

# Useless assignment inside a block (not inside a def)
describe "something" do
  it "does something" do
    problem = create(:problem)
    ^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `problem`.
    expect(true).to eq(true)
  end
end

# Useless assignment in sibling block — each `it` block is an independent
# closure. A variable written in one sibling is NOT accessible in another.
describe "matching tokens" do
  it "uses token" do
    token = FactoryBot.create(:access_token)
    expect(last_token).to eq(token)
  end
  it "does not use token" do
    token = FactoryBot.create(:access_token)
    ^^^^^ Lint/UselessAssignment: Useless assignment to variable - `token`.
    last_token = described_class.matching_token_for(application)
    expect(last_token).to eq(nil)
  end
end

# Useless in one sibling, used in another (only the unused one is flagged)
RSpec.describe "examples" do
  context "first" do
    result = compute_something
    ^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `result`.
    expect(true).to be(true)
  end
  context "second" do
    result = compute_something
    use(result)
  end
end

# Useless assignment inside a lambda block
describe "lambda with unused var" do
  it "does not use val" do
    callback = ->(x) {
      val = x * 2
      ^^^ Lint/UselessAssignment: Useless assignment to variable - `val`.
      puts "done"
    }
    callback.call(5)
  end
end

# Deeply nested sibling blocks — each `it` is still independent
describe "outer" do
  context "inner" do
    it "first" do
      data = fetch_data
      ^^^^ Lint/UselessAssignment: Useless assignment to variable - `data`.
      expect(true).to eq(true)
    end
    it "second" do
      data = fetch_data
      use(data)
    end
  end
end

# Reassigned after read — the last assignment is useless
def reassigned_after_read
  foo = 1
  puts foo
  foo = 3
  ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
end

# First assignment overwritten before read
def overwritten_before_read
  foo = 1
  ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  foo = 3
  puts foo
end

# Multiple reassignments, all but last read are useless
def multiple_reassign
  foo = 1
  ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  bar = 2
  foo = 3
  ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  puts bar
end

# Top-level useless assignment
foo = 1
^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
bar = 2
puts bar

# Assignment in single-branch if, unreferenced
def single_branch_if(flag)
  if flag
    foo = 1
    ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  end
end

# Assignment in if branch unreferenced, else branch also unreferenced
def both_branches_unused(flag)
  if flag
    foo = 2
    ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  else
    foo = 3
    ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  end
end

# Useless assignment in loop body
def useless_in_loop
  while true
    foo = 1
    ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  end
end

# Reassigned in same branch — first is useless
def reassigned_same_branch(flag)
  if flag
    foo = 1
    ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
    foo = 2
  end
  foo
end

# Unreferenced assignment before reassignment in if branch
def useless_before_branch_reassign(flag)
  foo = 1
  ^^^ Lint/UselessAssignment: Useless assignment to variable - `foo`.
  if flag
    foo = 2
    puts foo
  end
end

# For loop variable unreferenced
for item in items
    ^^^^ Lint/UselessAssignment: Useless assignment to variable - `item`.
end

# Modifier-if reassignment after a prior write: both writes are useless.
begin
  pwn_provider = 'ruby-gem'
  ^^^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `pwn_provider`.
  pwn_provider = ENV.fetch('PWN_PROVIDER') if ENV.keys.any? { |s| s == 'PWN_PROVIDER' }
  ^^^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `pwn_provider`.
end

# Each unread case branch assignment is its own offense.
case option
when :R
  track_data = read_card
  ^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `track_data`.
when :B
  track_data = backup_card
  ^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `track_data`.
when :L
  track_data = PWN::Plugins::MSR206.load_card_from_file(
  ^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `track_data`.
    msr206_obj: msr206_obj
  )
end

# Sequential optional branches that assign the same variable keep both writes.
if api_version == 'v1'
  tests_by_engagement_object = test_list[:objects].select do |test|
  ^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `tests_by_engagement_object`.
    test[:engagement] == engagement_resource_uri
  end
end

if api_version == 'v2'
  tests_by_engagement_object = test_list[:results].select do |test|
  ^^^^^^^^^^^^^^^^^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `tests_by_engagement_object`.
    test[:engagement] == engagement_resource_uri
  end
end

exec_resp = PWN::Plugins::MSR206.exec(
^ Lint/UselessAssignment: Useless assignment to variable - `exec_resp`.

is_found ? found += [c] : found
           ^^^^^ Lint/UselessAssignment: Useless assignment to variable - `found`.

is_found ? found += [c] : found
           ^^^^^ Lint/UselessAssignment: Useless assignment to variable - `found`.

# FN fix: sibling-branch reads must not keep an exclusive assignment alive.
def unformat_guid(guid)
  if guid.length == 22
    guid = ifc_guid_to_hex(guid)
    ^^^^ Lint/UselessAssignment: Useless assignment to variable - `guid`.
  else
    guid.tr('-', '')
  end
end

# FN fix: sequential overwrites before block capture — the variable is later
# captured by a block, but earlier overwrites are still useless.
def sequential_then_block_capture
  exec_resp = perform_action(:red_off)
  ^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `exec_resp`.
  exec_resp = perform_action(:yellow_off)
  ^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `exec_resp`.
  exec_resp = perform_action(:green_on)
  items.each do |item|
    exec_resp = perform_action(item)
    puts exec_resp
  end
end

# FN fix: single overwrite before block capture
def single_overwrite_then_block
  result = compute_initial
  ^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `result`.
  result = compute_final
  [1, 2].each { |x| result = x }
  puts result
end

# An exclusive outer-branch read must not suppress the earlier rescue-clause
# offense in the rescue chain.
def rescue_chain_read_in_else(flag)
  if flag
    begin
      work
    rescue SomeError
      score = 0
      ^^^^^ Lint/UselessAssignment: Useless assignment to variable - `score`.
    rescue OtherError
      score = 99
      ^^^^^ Lint/UselessAssignment: Useless assignment to variable - `score`.
    end
  else
    puts score
  end
end

# Earlier rescue clauses in the same chain are separate sibling branches.
def multi_rescue_self_reference(sock_obj)
  begin
    work
  rescue Errno::ECONNRESET
    sock_obj = disconnect(sock_obj) unless sock_obj.nil?
    ^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `sock_obj`.
  rescue StandardError
    sock_obj = disconnect(sock_obj) unless sock_obj.nil?
    ^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `sock_obj`.
  end
end

# Outer rescue branches must not leak into nested proc-local variable scopes.
def proc_locals_inside_begin(chat)
  begin
    checker = proc {
      test_request = nil
      ^^^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `test_request`.
      test_response = nil
      ^^^^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `test_response`.

      ref_request = chat.copyRequest
      request, ref_response = doRequest(ref_request, :default => true)
      ^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `request`.

      test_request = chat.copyRequest

      test_request.setMethod 'POST'
      test_request.removeHeader 'Connection'
      test_request.setHeader 'Content-Type', 'application/x-www-form-urlencoded'
      test_request.setHeader 'Transfer-Encoding', 'chunked'

      smuggle = "GET /fourOfour.txt HTTP/1.1\r\nHost: localhost\r\n\r\n"
      body = "0\r\n" + smuggle

      test_request.setHeader 'Content-Length', "#{body.length}"
      test_request.setBody body

      te_index = test_request.index { |h| h =~ /Transfer\-Encoding/ }
      cl_index = test_request.index { |h| h =~ /Content\-Length/ }

      if te_index < cl_index
        dummy = test_request[te_index]
        test_request[te_index] = test_request[cl_index]
        test_request[cl_index] = dummy
      end

      test_request, test_response = doRequest(
        test_request,
        :no_connection_close => true,
        :update_contentlength => false
      )

      if test_response.status_code != ref_response.status_code then
        addFinding(
          test_request,
          test_response,
          :check_pattern => "#{body}",
          :proof_pattern => "#{test_response.status_code}",
          :chat => chat,
          :title => '[ HRS ]'
        )
      end
      [test_request, test_response]
    }
    yield checker
  rescue => bang
    puts bang
  end
end

# The last rescue clause should not be suppressed by its own RHS self-reference.
def final_rescue_assignment(flag)
  connection = connect(flag)
  begin
    work(connection)
  rescue TimeoutError
    handle_timeout
  rescue StandardError => e
    connection = disconnect(connection) unless connection.nil?
    ^^^^^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `connection`.
    raise e
  end
end

exec_resp = PWN::Plugins::MSR206.exec(
^ Lint/UselessAssignment: Useless assignment to variable - `exec_resp`.

# Unused rescue capture in a complete rescue chain.
def rescue_capture_unused
  begin
    work
  rescue Timeout::Error => e
                           ^ Lint/UselessAssignment: Useless assignment to variable - `e`.
    handle_timeout
  rescue StandardError => e
    puts e.message
  end
end

# Condition-only assignment is useless when the variable is not read in the body.
def if_condition_assignment_only
  if v = @options[:ban_except]
     ^ Lint/UselessAssignment: Useless assignment to variable - `v`.
    do_something
  end
end

# FN fix: a normal `if` condition assignment overwrites the initializer before
# any later read can consume it.
def if_condition_overwrites_initializer(input)
  origin = nil
  ^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `origin`.
  if origin = input
    puts origin
  end
end

# FN fix: `unless` condition assignments behave the same way.
def unless_condition_overwrites_initializer
  r = nil
  ^ Lint/UselessAssignment: Useless assignment to variable - `r`.
  unless r = @info[:db]
    puts "fallback"
  end
  puts r
end

# FN fix: the initializer stays dead across later predicate assignments in an
# `if`/`elsif` chain.
def if_elsif_condition_overwrites_initializer(command)
  output = ""
  ^^^^^^ Lint/UselessAssignment: Useless assignment to variable - `output`.
  if (output = roll_tables(command, TABLES))
    return output
  elsif (output = roll_tables(command, A2Z_TABLES))
    return output
  end
end
