@payloads ||= {
    ruby:   'sleep(__TIME__/1000);',
    php:    'sleep(__TIME__/1000);',
    perl:   'sleep(__TIME__/1000);',
    python: 'import time;time.sleep(__TIME__/1000);',
    java:   'Thread.sleep(__TIME__);',
    asp:    'Thread.Sleep(__TIME__);',
}
