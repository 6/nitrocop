# nitrocop-config: EnforcedStyle: strict

Time.now.zone

"Timezone: #{Time.now.zone}."

assert_match Time.now.zone.to_s, markup("datetime_attr")

puts "Time.now.zone => #{Time.now.zone.inspect}"

Time.to_mongo(Time.local(2009, 8, 15, 0, 0, 0)).zone

Time.new("2021-12-25 00:00:00").zone.should == Time.new(2021, 12, 25).zone
