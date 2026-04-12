items.each do |x|
  puts x
end

items.each { |x| puts x }

[1, 2].map do |x|
  x * 2
end

# end aligned with chain expression start (not the do-line)
@source_account.passive_relationships
               .where(account: Account.local)
               .in_batches do |follows|
  follows.update_all(target_account_id: 1)
end

# end aligned with call expression start in a hash value
def generate
  {
    data: items.map do |item|
            item.to_s
          end,
  }
end

# end aligned with call on previous line via backslash continuation
it 'does something' \
   'very interesting' do
  run_test
end

# end aligned with call on previous line via multiline args
option(opts, '--fail-level SEVERITY',
       RuboCop::Cop::Severity::NAMES) do |severity|
  @options[:fail_level] = severity
end

# end aligned with call expression that has multiline args ending with comma
add_offense(node,
            message: format(MSG,
                            flag: true)) do |corrector|
  corrector.replace(node, replacement)
end

# Multiline %i[] array with .each do block — end aligns with %i[
%i[opposite_style_detected unexpected_style_detected
   ambiguous_style_detected conflicting_styles_detected
   unrecognized_style_detected
   no_acceptable_style!].each do |method|
  puts method
end

# do...end block inside a brace block — end aligns with chain root
to = lambda { |env|
  hostess.call(env)
    .tap do |response|
      response[1].delete("x-cascade")
  end
}

# string concatenation with + on multiline method description (RSpec-style)
it "should convert " +
    "some value " +
    "correctly" do
  run_test
end

# string concatenation with + on multiline describe block
describe User, "when created with a name known to cause issues " +
    "in certain cases" do
  it "should work" do
    expect(true).to be true
  end
end

# block as argument inside parentheses — end aligns with method inside parens
expect(arr.all? do |o|
         o.valid?
       end)

# FP fix: multiline string literal (no explicit continuation) -- do on wrapped line
it "returns remote census api response when available and valid without send
    date_of_birth and postal_code" do
  response = api.call(1, "A", nil, nil)
  expect(response).to eq(true)
end

# FP fix: multiline args with comments between continuation lines
it "navigates correctly the path from the overview page to boards",
   # The polling interval is lowered for testing
   # In reality, it does not really matter
   with_settings: { notifications_polling_interval: 1_000 } do
  visit project_path(project)
  expect(true).to eq(true)
end

# FP fix: multiline string with backslash continuation spanning many lines
describe "WHEN the user is allowed to update entries " \
         "WHEN updating:
            entity_type
            entity_id
            user_id
            units
            cost_type
            spent_on" do
  let(:expected_entity) { build(:entity) }
end

# FP fix: describe block with multiline arg and comma before do
add_api_endpoint "API::V3::Users::UsersAPI", :id,
                 caption: ->(*) { I18n.t("label") },
                 if: ->(*) { enabled? },
                 icon: "image" do
  mount ::API::V3::Users::UserAvatarAPI
end

# FP fix: chained block end aligns with method name in assignment context
response = stub_comms do
             verify_something
           end.check_request do |data|
  assert_match(/pattern/, data)
end.respond_with(response)

# FP fix: end&.path aligns with method name in assignment context
tmpl_path = caller_locations(1, 2).find do |loc|
              loc.label.include?("method_missing").!
            end&.path

# FP fix: && on same line as do — end aligns with LHS of && expression
next true if urls&.size&.positive? && urls&.all? do |url|
               url.include?(T.must(cred["registry"]))
             end


# Lambda/proc brace block } aligns with -> start or line indent
scope :last_n_per_feed, -> (n, feed_ids) {
   ranked_posts = select(select_sql)
   from(ranked_posts, "entries")
     .where("entries_rank <= ?", n)
     .where(feed_id: feed_ids)
}

# Lambda brace block } aligns with -> start or line indent
[:favorite_even_number, validate_with: -> (v) {
   unless v.nil? || v.even?
     {code: :even, msg: "Value must be even. Was: #{v}"}
   end
}]

# FP fix: } chained via next-line dot (not immediately after })
victims = replicas.select {
            !(it.destroy_set? || it.strand.label == "destroy")
          }
  .sort_by { |r| r.created_at }

# FP fix: do..end block in if condition with && — end aligns with LHS of && expression
if adjustment_type == "removal" && article.tag_list.none? do |tag|
     tag.casecmp(tag_name).zero?
   end
  errors.add(:tag_id, "not live")
end

# FP fix: multiline assignment on previous line — end aligns with assignment LHS
packages_lines, last_package_lines =
  stdout
  .each_line
  .map(&:strip)
  .reject { |line| end_of_lines?(line) }
  .reduce([[], []]) do |(pkgs, pkg), line|
  if start?(line)
    pkgs.push(pkg) unless pkg.empty?
    [pkgs, [line]]
  else
    pkg.push(line)
    [pkgs, pkg]
  end
end

# FP fix: do on continuation line of multi-line call with bracket-key assignment
          env[:machine].id = env[:machine].provider.driver.clone_vm(
            env[:clone_id], options) do |progress|
            env[:ui].clear_line
          end

# FP fix: do on continuation line with multi-line args (ask/lambda pattern)
  entry[:phone] = ask("Phone?  ",
                      lambda { |p| p.to_s }) do |q|
    q.validate = true
  end

# FP fix: << operator on same line — end aligns with LHS of <<
      lists << tag.ul(:class => "pagination") do
        items.each do |page|
          concat page
        end
      end

# FP fix: << with multi-line args — end aligns with acc
              acc << HexaPDF::Layout::InlineBox.create(width: 10, height: 10,
                                                       margin: [0, 1, -2, 0]) do |cv, box|
                cv.image("icon.png", at: [0, 0], width: box.content_width)
              end

# FP fix: rescue modifier around assignment RHS in logical operand
if (command = ENV["DOT"] || "dot") &&
   (result = build_value { |value|
               value
             } rescue false)
  puts result
end

# FP fix: splat wrapper aligns } with the splat, not the call start
wrap *items.map { |item|
     item
     }

# FP fix: plain ||= memoization still aligns end with the assignment lhs
@connection ||= Faraday.new(url: base_url,
                             ssl: ssl_options) do |conn|
  conn.request :json
end

# FP fix: if the line starts with a previous closer before ||, keep the RHS block target
left_side.find {
  it
} || right_side.any? do |item|
  item
     end

# FP fix: multi-assignment with leading-dot chain still aligns with the lhs
a, b, c = create_list(:form_submission, 3, user_account: user_account)
           .zip(%w[A B C])
           .map do |submission, form_type|
             submission.update(form_type: form_type)
             submission
end

# FP fix: chained send after end can stop at the surrounding masgn lhs
changed_playlist, new, changed_position, unchanged = playlist.items.
  sort_by(&:position).
  group_by do |item|
    if item.playlist_id_was != item.playlist_id
      :changed_playlist
    elsif item.position_was.nil?
      :new
    elsif item.position_was != item.position
      :changed_position
    else
      :unchanged
    end
end.values_at(:changed_playlist, :new, :changed_position, :unchanged).map(&:to_a)

# FP fix: chained [] after end should stop at the preceding send, not the assignment lhs
entity_type = config.select do |_key, value|
                value.any? do |bp|
                  bp == bp_code
                end
              end.keys[0]

# FP fix: lambda call used as an optional argument keeps the lambda column
def print_tree(level = node_depth, max_depth = nil,
               block = lambda { |node, prefix|
                         puts "#{prefix} #{node.name}"
                       })
end

# FP fix: stabby lambda inside || should align with the logical expression start
auth.provided? && auth.basic? && auth.credentials &&
  (conf.auth[:logic] || ->(data, u, p) {
    %w(admin password) == [u, p]
   }).(data, *auth.credentials)

# FP fix: chained end.transpose in a lambda body still aligns with the masgn lhs
-> (genotypes) do
  print "Fits: "
  fits, parall_infos = Parallel.map(0...genotypes.shape.first,
      in_processes: nprocs, isolation: true) do |i|
    env = parall_envs[i]
    fits = ntrials.times.map { fitness_one genotypes[i, true], env: env }
    fit = fits.to_na.mean
    print "[m#{fit}] "
    [fit, compr.parall_info]
  end.transpose
end
