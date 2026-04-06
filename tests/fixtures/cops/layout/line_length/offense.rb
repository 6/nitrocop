x = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                                                                                                                        ^^^^^ Layout/LineLength: Line is too long. [125/120]
y = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                                                                                                                        ^^^^^^^^^^ Layout/LineLength: Line is too long. [130/120]
z = "ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                                                                                                                        ^ Layout/LineLength: Line is too long. [121/120]

																				aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
                                                                                                    ^ Layout/LineLength: Line is too long. [140/120]

# A commented string concatenation like <<'taint_tracer.js...' must not open a fake heredoc.
# RuboCop still checks the later long lines.
# expect(subject.digest).to eq('pt.browser.arachni/' <<'taint_tracer.js><SCRIPT src' <<
x = [
                                                       "function( name, value ){\n            document.cookie = name + \"=post-\" + value\n        }",
                                                                                                                        ^ Layout/LineLength: Line is too long. [150/120]
]

# expect(subject.elements_with_events).to eq('pt.browser.arachni/' <<'taint_tracer.js><SCRIPT src' <<
click_handlers = {
  "click" => [
                                "function( e ) {\n\t\t\t\t// Discard the second event of a jQuery.event.trigger() and\n\t\t\t\t// when an event is called after a page has unloaded\n\t\t\t\treturn typeof jQuery !== core_strundefined && (!e || jQuery.event.triggered !== e.type) ?\n\t\t\t\t\tjQuery.event.dispatch.apply( eventHandle.elem, arguments ) :\n\t\t\t\t\tundefined;\n\t\t\t}"
                                                                                                                        ^ Layout/LineLength: Line is too long. [382/120]
  ]
}

         "It's based on the following blog post: [https://medium.com/gett-engineering/rxswift-to-apples-combine-cheat-sheet-e9ce32b14c5b](https://medium.com/gett-engineering/rxswift-to-apples-combine-cheat-sheet-e9ce32b14c5b)\n\n"
                                                                                                                        ^ Layout/LineLength: Line is too long. [230/120]

      lines << %{<svg width="#{width}" height="#{height}" version="1.1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">}
                                                                                                                        ^ Layout/LineLength: Line is too long. [151/120]

        5.times { |index| results << Hashie::Mash::Rash.new(title: "title #{index}", description: "content #{index}", path: "http://foo.gov/#{index}", changed: '2020-09-09 00:00:00 UTC', created: '2020-09-09 00:00:00 UTC', thumbnail_url: 'https://search.gov/img.svg') }
                                                                                                                        ^ Layout/LineLength: Line is too long. [269/120]

        5.times { |index| results << Hashie::Mash::Rash.new(title: "title #{index}", description: "content #{index}", path: "http://foo.gov/#{index}") }
                                                                                                                        ^ Layout/LineLength: Line is too long. [152/120]

      5.times { |index| results << Hashie::Mash::Rash.new(title: "title #{index}", description: "content #{index}", url: "http://foo.gov/#{index}") }
                                                                                                                        ^ Layout/LineLength: Line is too long. [149/120]

        5.times { |index| results << Hashie::Mash::Rash.new(title: "title #{index}", description: "content #{index}", url: "http://foo.gov/#{index}", published_at: twelve_years_ago, youtube_thumbnail_url: "http://youtube.com/#{index}", duration: '1:23') }
                                                                                                                        ^ Layout/LineLength: Line is too long. [255/120]

final_answer = "Here are some fascinating facts from the James Webb Space Telescope's Wikipedia page" # rubocop:disable Layout/LineLength
expect(agent.conversation_history).to eq([
          { "role" => "user", "content" => "Tell me some interesting facts from the James Webb Space Telescope's Wikipedia page" },
                                                                                                                        ^^^^^^^^^^^ Layout/LineLength: Line is too long. [131/120]
])

        raise Puppet::Network::HTTP::Error::HTTPNotFoundError.new("No route for #{req.method} #{req.path}", Puppet::Network::HTTP::Issues::HANDLER_NOT_FOUND)
                                                                                                                        ^ Layout/LineLength: Line is too long. [157/120]

raise Puppet::Network::HTTP::Error::HTTPNotFoundError.new(
                                                                  "  Supported /puppet API versions: #{Puppet::Network::HTTP::SERVER_URL_VERSIONS}\n",
                                                                                                                        ^ Layout/LineLength: Line is too long. [150/120]
                                                                  Puppet::Network::HTTP::Issues::HANDLER_NOT_FOUND
)

      raise Puppet::Network::HTTP::Error::HTTPBadRequestError.new(_("Missing required Accept header"), Puppet::Network::HTTP::Issues::MISSING_HEADER_FIELD)
                                                                                                                        ^ Layout/LineLength: Line is too long. [155/120]

    decoded  = '200904281000001|DOM|IND|INR|58.00|22|NULL|http://localhost:3000/orders/1/ed5230696ad525b9e322a6a64b56322e/done?utm_nooverride=1|http://hardcoregamer.localhost:3000|TOML'
                                                                                                                        ^ Layout/LineLength: Line is too long. [185/120]

    expected = '200904281000001|DOM|IND|INR|58.00|22|1|http://localhost:3000/orders/1/ed5230696ad525b9e322a6a64b56322e/done?utm_nooverride=1|http://hardcoregamer.localhost:3000|TOML'
                                                                                                                        ^ Layout/LineLength: Line is too long. [182/120]

			@checks ||= self.available_checks.collect { |c| perform_check c }.flatten(1) + children.collect(&:checks).flatten(1)
                                                                                                                     ^ Layout/LineLength: Line is too long. [122/120]

  # Encapsulates data returned from the {APN Feedback Service}[http://developer.apple.com/iphone/library/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/CommunicatingWIthAPS/CommunicatingWIthAPS.html#//apple_ref/doc/uid/TP40008194-CH101-SW3].
                                                                                                                        ^ Layout/LineLength: Line is too long. [260/120]

  # When supplied with the certificate path and the desired environment, connects to the {APN Feedback Service}[http://developer.apple.com/iphone/library/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/CommunicatingWIthAPS/CommunicatingWIthAPS.html#//apple_ref/doc/uid/TP40008194-CH101-SW3]
                                                                                                                        ^ Layout/LineLength: Line is too long. [308/120]

      expect(result).to include('<a href="https://www.findagrave.com/cgi-bin/fg.cgi?page=gr&amp;GSln=test&amp;GRid=123456">https://www.findagrave.com/cgi-bin/fg.cgi?page=gr&amp;GSln=test&amp;GRid=123456</a>')
                                                                                                                        ^ Layout/LineLength: Line is too long. [208/120]

      expect(@article.bibliography).to include('<a href="https://www.findagrave.com/cgi-bin/fg.cgi?page=gr&amp;GSln=test&amp;GRid=123456">https://www.findagrave.com/cgi-bin/fg.cgi?page=gr&amp;GSln=test&amp;GRid=123456</a>')
                                                                                                                        ^ Layout/LineLength: Line is too long. [223/120]

        expect(@event.tags.last.machine_tag.url).to eq "http://my-calagator.org/defunct?url=https://web.archive.org/web/#{event_date}/http://upcoming.yahoo.com/event/1234"
                                                                                                                        ^ Layout/LineLength: Line is too long. [171/120]

        expect(@venue.tags.last.machine_tag.url).to eq "http://my-calagator.org/defunct?url=https://web.archive.org/web/#{venue_date}/http://upcoming.yahoo.com/venue/1234"
                                                                                                                        ^ Layout/LineLength: Line is too long. [171/120]

        artist_commentary_desc: "今週のらくがき\n\nhttps://downloads.fanbox.cc/images/post/209386/Q8rZ0iMHpcmJDACEzNGjTj9E.jpeg\n水着BBちゃん\nhttps://downloads.fanbox.cc/images/post/209386/8dRNHXkFqAwSt31W2Bg8fSdL.jpeg\n第一再臨もセクシー\n",
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [222/120]

        @artist1 = create(:artist, name: "aaa", group_name: "g_aaa", other_names: "111 222", url_string: "https://x.com/111\n-https://x.com/222")
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [145/120]

        @artist2 = create(:artist, name: "bbb", other_names: "111 333", url_string: "https://x.com/111\n-https://x.com/333\nhttps://x.com/444")
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [143/120]

          "Documentation: [https://developer.github.com/v3/repos/contents/#create-a-file](https://developer.github.com/v3/repos/contents/#create-a-file)."
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [154/120]

          "More information about how the lane context works: [https://docs.fastlane.tools/advanced/#lane-context](https://docs.fastlane.tools/advanced/#lane-context)."
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [168/120]

        "More information: [http://tryouts.readthedocs.org/en/latest/releases.html#create-release](http://tryouts.readthedocs.org/en/latest/releases.html#create-release)"
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [170/120]

RECT_WHITELIST = [:grid, :g, :l, :left, :x, :fl, :top, :t, :y, :w, :width, :h, :height, :below_prev, :bp, :below_previous, :left_of_prev, :lop, :left_of_previous, :r, :right, :b, :bottom, :from_righ...
                                                                                                                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/LineLength: Line is too long. [201/120]

*            *              |    * |--+--from_right---|*    {l: #{i_f_to_s(left_in_root_view)}, t: #{i_f_to_s(top_in_root_view)},
                                                                                                                        ^^^^^^^^^ Layout/LineLength: Line is too long. [129/120]
