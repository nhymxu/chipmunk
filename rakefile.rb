
require 'fileutils'
require 'json'

module OS
  def OS.windows?
    (/cygwin|mswin|mingw|bccwin|wince|emx/ =~ RUBY_PLATFORM) != nil
  end

  def OS.mac?
   (/darwin/ =~ RUBY_PLATFORM) != nil
  end

  def OS.unix?
    !OS.windows?
  end

  def OS.linux?
    OS.unix? and not OS.mac?
  end

  def OS.jruby?
    RUBY_ENGINE == 'jruby'
  end
end

DIST_FOLDER = "application/electron/dist/"
COMPILED_FOLDER = "application/electron/dist/compiled/"
RELEASE_FOLDER = "application/electron/dist/release/"
INCLUDED_PLUGINS_FOLDER = "application/electron/dist/compiled/plugins/"
INCLUDED_APPS_FOLDER = "application/electron/dist/compiled/apps/"
APP_PACKAGE_JSON = "application/electron/package.json"

if OS.windows? == true
  TARGET_PLATFORM_NAME = "win64"
  TARGET_PLATFORM_ALIAS = "win"
elsif OS.mac? == true
  TARGET_PLATFORM_NAME = "darwin"
  TARGET_PLATFORM_ALIAS = "mac"
else
  TARGET_PLATFORM_NAME = "linux"
  TARGET_PLATFORM_ALIAS = "linux"
end

puts "Detected target platform is: #{TARGET_PLATFORM_NAME} / #{TARGET_PLATFORM_ALIAS}"

desc "quick build after update"
task :quick do
  cd "application/client.core" do
    sh "npm update logviewer.client.toolkit"
  end
  cd "application" do
    sh "jake client:all --skip-npm"
    sh "jake electron:quick --skip-npm"
  end
  rm_rf "~/.logviewer"
end

desc "start"
task :start do
  cd "application/electron" do
    sh "npm run electron"
  end
end

desc "prepare"
task :prepare do
  puts "Installing npm libs, which is needed for installing / updateing process"
  sh "npm install typescript jake --global"
end

desc "folders"
task :folders do
  PATHS = [DIST_FOLDER, COMPILED_FOLDER, RELEASE_FOLDER, INCLUDED_PLUGINS_FOLDER, INCLUDED_APPS_FOLDER] 
  i = 0;
  while i < PATHS.length
    path = PATHS[i]
    puts "Check / create folder: #{path}"
    Dir.mkdir(path) unless File.exists?(path)
    i += 1
  end
  Rake::Task["folders"].reenable
end

desc "install"
task :install do
  Rake::Task["folders"].invoke
  cd "application" do
    puts "Installing: root"
    sh "npm install"
  end
  cd "application/client.core" do
    puts "Installing: core"
    sh "npm install"
  end
  cd "application/client.libs/logviewer.client.components" do
    puts "Installing: components"
    sh "npm install"
  end
  cd "application/client.plugins" do
    puts "Installing: plugins env"
    sh "npm install"
  end
  cd "application/electron" do
    puts "Installing: electron"
    sh "npm install"
    sh "npm run build-ts"
  end
  cd "application" do
    puts "Building: client"
    sh "jake client:all --skip-npm"
    puts "Building: electron"
    sh "jake electron:quick --skip-npm"
  end
end

desc "install plugins"
task :plugins do
  puts "Drop included plugins: #{INCLUDED_PLUGINS_FOLDER}"
  FileUtils.rm_r(INCLUDED_PLUGINS_FOLDER) unless !File.exists?(INCLUDED_PLUGINS_FOLDER)
  Rake::Task["folders"].invoke
  cd "application/client.plugins.standalone/row.parser.ascii" do
    puts "Install plugin: row.parser.ascii"
    sh "npm install"
  end
  cd "application/sandbox/dlt/process" do
    puts "Install plugin: dlt"
    sh "npm install"
    sh "npm install electron@4.0.3 electron-rebuild@^1.8.2"
    sh "./node_modules/.bin/electron-rebuild"
    sh "npm uninstall electron electron-rebuild"
  end
  cd "application/sandbox/serial/process" do
    puts "Install plugin: serial"
    sh "npm install"
    sh "npm install electron@4.0.3 electron-rebuild@^1.8.2"
    sh "./node_modules/.bin/electron-rebuild"
    sh "npm uninstall electron electron-rebuild"
  end
  cd "application/sandbox/processes/process" do
    puts "Install plugin: processes"
    sh "npm install"
    sh "npm install electron@4.0.3 electron-rebuild@^1.8.2"
    sh "./node_modules/.bin/electron-rebuild"
    sh "npm uninstall electron electron-rebuild"
  end
  cd "application/sandbox/xterminal/process" do
    puts "Install plugin: xterminal"
    sh "npm install"
    sh "npm install electron@4.0.3 electron-rebuild@^1.8.2"
    sh "./node_modules/.bin/electron-rebuild"
    sh "npm uninstall electron electron-rebuild"
  end
  cd "application" do
    puts "Build: all libs"
    sh "jake client:all --skip-npm"
    puts "Build: all plugins"
    sh "jake plugins:all --skip-npm"
  end
end

desc "update indexer"
task :updateindexer do
  cd "application/electron" do
    puts "Updating indexer"
    sh "npm uninstall logviewer.lvin"
    sh "npm install logviewer.lvin@latest"
  end
end

desc "update toolkit"
task :updatetoolkit do
  cd "application/client.core" do
    puts "Update toolkits for: core"
    sh "npm uninstall logviewer.client.toolkit"
    sh "npm install logviewer.client.toolkit@latest"
  end
  cd "application/client.plugins" do
    puts "Update toolkits for: angular plugins"
    sh "npm uninstall logviewer.client.toolkit"
    sh "npm install logviewer.client.toolkit@latest"
  end
  cd "application/client.plugins.standalone/row.parser.ascii" do
    puts "Update toolkits for: none-angular plugins"
    sh "npm uninstall logviewer.client.toolkit"
    sh "npm install logviewer.client.toolkit@latest"
  end
  cd "application" do
    puts "Rebuild: client"
    sh "jake client:all --skip-npm"
    puts "Rebuild: plugins"
    sh "jake plugins:all --skip-npm"
    puts "Rebuild: electron"
    sh "jake electron:quick --skip-npm"
  end
end

desc "build launcher"
task :buildlauncher do
  Rake::Task["folders"].invoke

  SRC_APP_DIR = "application/apps/updater/target/release/"
  APP_FILE = "launcher"
  if OS.windows? == true
    APP_FILE = "launcher.exe"
  end

  cd "application/apps/updater" do
    puts 'Build updater'
    sh "cargo build --release"
  end

  puts "Check old version of app: #{INCLUDED_APPS_FOLDER}#{APP_FILE}"
  FileUtils.rm("#{INCLUDED_APPS_FOLDER}#{APP_FILE}") unless !File.exists?("#{INCLUDED_APPS_FOLDER}#{APP_FILE}")
  puts "Updating app from: #{SRC_APP_DIR}#{APP_FILE}"
  FileUtils.cp("#{SRC_APP_DIR}#{APP_FILE}", "#{INCLUDED_APPS_FOLDER}#{APP_FILE}")

end

desc "full update"
task :update => [:updateindexer, :updatetoolkit, :buildlauncher]

desc "build"
task :build do

  FileUtils.rm_r(RELEASE_FOLDER) unless !File.exists?(RELEASE_FOLDER)
  Rake::Task["folders"].invoke

  cd "application/electron" do
    sh "npm run build-ts"
    sh "./node_modules/.bin/build --#{TARGET_PLATFORM_ALIAS}"
  end
end

desc "Prepare package to deploy on Github"
task :prepare_to_deploy do
  puts "Reading package file"
  package_str = File.read(APP_PACKAGE_JSON)
  package = JSON.parse(package_str)
  puts "Detected version: #{package["version"]}"
  cd "application/electron/dist/release" do
    release_name = "chipmunk@#{package["version"]}-#{TARGET_PLATFORM_NAME}-portable"
    case TARGET_PLATFORM_ALIAS
      when "mac"
        cd "mac" do
          sh "tar -cvzf ../#{release_name}.tgz ./chipmunk.app"
        end
      when "linux"
        cd "#{TARGET_PLATFORM_ALIAS}-unpacked" do
          sh "tar -cvzf ../#{release_name}.tgz *"
        end
      when "win"
        cd "#{TARGET_PLATFORM_ALIAS}-unpacked" do
          sh "tar -cvzf ../#{release_name}.tgz ./*"
        end
    end
  end
end

desc "Build the full build pipeline for a given platform"
task :full_pipeline do
  Rake::Task["install"].invoke
  Rake::Task["update"].invoke
  Rake::Task["plugins"].invoke
  Rake::Task["build"].invoke
  Rake::Task["prepare_to_deploy"].invoke
end