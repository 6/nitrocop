items.each do |x|
  puts x
  end
  ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
items.map do |x|
  x * 2
    end
    ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
[1, 2].select do |x|
  x > 1
      end
      ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: end aligns with RHS of assignment (call expression) instead of LHS
answer = prompt.select("Pick one") do |menu|
           menu.choice "A"
         end
         ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: brace block } misaligned — } at col 4, lambda{ at col 8
req = Rack::MockRequest.new(
  show_status(
        lambda{|env|
          env["rack.showstatus.detail"] = "gone too meta."
          [404, { "content-type" => "text/plain", "content-length" => "0" }, []]
    }))
    ^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.
# FN: do..end block misaligned in multi-arg call
assert_called_with(
  ActiveRecord::Tasks::DatabaseTasks, :structure_dump,
  ["task_dump",
   "--result-file",
   filename]
) do
        ActiveRecord::Tasks::DatabaseTasks.structure_dump(
          @configuration.merge("sslca" => "ca.crt"),
          filename)
        end
        ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: lambda do..end block misaligned
let(:app) do
   ->(_) do
    [200, {}, "ok"]
  end
  ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
end
# FN: lambda brace block } misaligned
-> {
  m_that_does_not_use_block { }
    }.should complain("warning")
    ^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.
# FN: accepted_states.any? block end misaligned (off by 2)
accepted_states.any? do |(status, reason)|
  if reason.nil?
    payment[:payment_status] == status
  end
    end
    ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: end misaligned in expect(auditable.audit do ... end) — Arachni pattern
                        expect(auditable.audit( {},
                                          format: [ Format::STRAIGHT ] ) do |_, element|
                            injected << element.affected_input_value
                        end).to be_nil
                        ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: } misaligned in deep brace block (seyhunak pattern)
      expect(element).to have_tag(:div,
        with: {class: "alert"}) {
          have_tag(:button,
            text: "x"
          )

      }
      ^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.
# FN: end misaligned by 1 with %w literal (floere pattern)
%w[cpu object].each do |thing|
  profile thing do
    10_000.times { method }
  end
 end
 ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: lambda brace } misaligned with -> (refinery pattern)
  ->{
    page.within_frame do
      select_upload
    end
    }
    ^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.
# FN: end misaligned in combos block (bloom-lang pattern)
    result <= (sem_hist * use_tiebreak * tc).combos(sem_hist.from => use_tiebreak.from,
                                                     sem_hist.to => tc.from,
                                                     sem_hist.from => tc.to) do |s,t,e|
      [s.to, t.to]
    end
    ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: cross-line single assignment — end at col 8 misaligned with Class.new at col 10
        @connection_model =
          Class.new(Blazer::Connection) do
            def self.name
              "SnowflakeAdapter"
            end
        end
        ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: cross-line assignment with bracket LHS — end at col 10 misaligned with data at col 12
          body[:available][:items] =
            data[:networks].map do |id, network|
              {type: "app", name: network[:name]}
          end
          ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: cross-line assignment — end at col 8, File.open at col 10
          contents =
            File.open("config/setup.rb") do |src|
            src.read
          end
          ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: cross-line || continuation — end at col 0, but call at col 2
a ||
  items.each do |x|
  process(x)
end
^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: cross-line && continuation — end at col 0, but call at col 2
(value.is_a? Array) &&
  value.all? do |subvalue|
  type_check(subvalue)
end
^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
# FN: chained }.to_json in assignment should still align with the assignment LHS
result = items.map { |item|
         }.to_json
         ^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.

# FN: same-line || wrapper should not let end align with the inner call expression
def changed?
  to_be_destroyed.any? || proxy_target.any? do |record|
    record.changed?
                          end
                          ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
end

# FN: hash literal receiver should align end with `{` / `}.each do`, not just after `}`
{
  "Ab$9" => 4,
  "blah" => -2
}.each do |password, bonus_bits|
  puts password
 end
 ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.

# FN: splat wrapper should align end with `*`, not `FileList`
rdoc.rdoc_files.include(
  *FileList.new("*") do |list|
     list.exclude("TODO")
   end.to_a)
   ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.

# FN: << wrapper should align end with lhs, not the inner call
out << sequence.each_with_object(+'') do |col_name, s|
  s << col_name.to_s
       end
       ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.

# FN: << wrapper should align } with lhs, not the inner call
def handle_message(msg, connection = {})
  if request?(msg)
    tp << ThreadPoolJob.new(msg) { |i|
      handle_request(i, false, connection)
          }
          ^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.
  end
end

# FN: ||= chain should not align end with assignment lhs here
def link_options
  @link_options ||= pages.published.pluck(:name, :slug)
    .each_with_object(DEFAULT_LINKS.dup) do |(name, slug), memo|
    memo[name] = slug
  end.sort_by { |_key, value| navigation_links.index(value) || 0 }.to_h
  ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.
end

# FN: multiline stabby lambda should align end with `->` or the do-line indent
          scope :_candlestick, -> (timeframe: '1h',
                           segment_by: segment_by_column,
                           time: time_column,
                           volume: 'volume',
                           value: value_column) do
             select(time)
          end
          ^^^ Layout/BlockAlignment: Align `end` with the start of the line where the block is defined.

# FN: lambda brace block in keyword arg should not align with assignment lhs
s = read_from_minibuffer("prompt",
                         completion_proc: ->(value) {
  value
})
^ Layout/BlockAlignment: Align `}` with the start of the line where the block is defined.
