# nitrocop-config: EnforcedStyle: table, table, ignore_implicit
assert_equal({
               :controller => "cms/content",
               :action => "show_page_route",
               :requirements => {
                 :year => /\d{4,}/
               }, :conditions => {
                 :method => :get
               }
             }, route.options_map)
