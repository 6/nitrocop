# frozen_string_literal: true

x = 1
y = 2
puts "hello world"
a_long_variable_name = some_method_call(arg1, arg2, arg3)

# AllowURI: a URI that extends to end of line should be allowed even if line > 120
# (the default AllowURI: true makes this OK)
some_long_variable = "see https://example.com/very/long/path/that/pushes/the/line/over/the/limit/but/extends/to/end"

# AllowQualifiedName: a qualified name (Foo::Bar::Baz) that extends to end of line should be allowed
text_document: LanguageServer::Protocol::Interface::OptionalVersionedTextDocumentIdentifier.new(

# AllowHeredoc: long lines inside a single heredoc should be allowed
msg = <<~TEXT
  This is a very long line inside a heredoc that exceeds the default maximum line length of one hundred and twenty characters easily
TEXT

# AllowHeredoc: multiple heredocs opened on the same line — content of BOTH should be allowed
expect(<<~HTML.chomp.process.first).to eq(<<~TEXT.chomp)
  <p>This is a very long HTML line inside the first heredoc that exceeds the default maximum line length of one hundred and twenty characters easily</p>
HTML
  This is a very long text line inside the second heredoc that exceeds the default maximum line length of one hundred and twenty characters without issue
TEXT

# AllowURI: URL with embedded URL in query params — the first URL starts before max and extends to end of line
      "oembed_get_request" => "http://www.flickr.com/services/oembed/?format=json&frame=1&iframe=1&maxheight=420&maxwidth=420&url=http://www.flickr.com/photos/bees/2341623661",

# Legacy block disable syntax with `Layout:LineLength` should still suppress the enclosed long line
# rubocop:disable Layout:LineLength
x = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
# rubocop:enable Layout:LineLength

# AllowURI: URL followed by \n escape — the URL extends to end of line via word boundary extension
puts "\nFor all saved admins, copy and paste into the wiki at https://wiki.transformativeworks.org/mediawiki/AO3_Admins:\n"

# AllowURI: URL followed by :\n\n" — the URL extends to end of line via word boundary extension
  say_status "", "The easiest way of launching Elasticsearch is by running it with Docker (https://www.docker.com/get-docker):\n\n"

# AllowURI: URL in backticks followed by :\n — brace extension reaches end of line
        let(:message)         { "You have exceeded the rate limit. `https://www.hipchat.com/docs/apiv2/rate_limiting`:\nResponse: #{body.to_json}" }

# AllowURI: URLs with HTML entity &#46; inside array access — # in &#46; is not a real fragment
            expect(click_event.click_url).to eq(["https://www&#46;gumroad&#46;com/checkout", "https://seller&#46;gumroad&#46;com/l/abc"][i])

# AllowURI: URLs with real # fragments separated by single-quote separators — Ruby regex merges them
      vocab.sample_controlled_vocab_terms << FactoryBot.build(:sample_controlled_vocab_term, label: 'Mother',iri:'http://ontology.org/#mother',parent_iri:'http://ontology.org/#parent')

# AllowURI: URLs with IPv6 brackets and comma separator — Ruby regex merges them into one long match
      assert_equal 2, instance.get_connection_options("https://john:password@[2404:7a80:d440:3000:192a:a292:bd7f:ca19]:443/elastic/,http://host2")[:hosts].length

# AllowURI: URLs with &#8203; (zero-width space entity) — # in &#8203; is not a real fragment
      markdown.should include "* PUT [https:&#8203;/&#8203;/api.sample.com&#8203;/members&#8203;/add](members_api/add-PUT.md)"

__END__
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
