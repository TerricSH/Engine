pub fn print_global_help() {
    println!(
        "Engine sandbox\n\n\
         Game project workflow:\n\
           sandbox project new <directory> [--name NAME] [--with-csharp]\n\
           sandbox project check <project> [--report PATH]\n\
           sandbox project import <project> <source-file> --id ID [--type TYPE] [--separate-primitives] [--no-bake-node-transforms]\n\
           sandbox project scene list <project>\n\
           sandbox project scene new <project> <scene-id> [--name NAME]\n\
           sandbox project scene rename <project> <old-id> <new-id>\n\
           sandbox project scene delete <project> <scene-id> [--replacement-startup ID]\n\
           sandbox project scene set-startup <project> <scene-id>\n\
           sandbox project cook <project>\n\
           sandbox project sync-script-api <project>\n\
           sandbox project build-scripts <project>\n\
           sandbox project build <project>\n\
           sandbox project run <project> [--headless] [--frames N] [--report PATH] [--stream-cells]\n\
           sandbox project editor <project>\n\n\
         Short aliases:\n\
           sandbox game <project> [--headless] [--frames N]\n\
           sandbox editor <project>"
    );
}

pub(super) fn print_project_help() {
    println!(
        "Game project commands:\n\
           new      create a portable project and starter scene\n\
           check    validate the manifest, scene, and source asset references\n\
           import   copy, register, and cook a mesh, texture, or material source\n\
           scene    list, create, and choose the startup scene\n\
           cook     cook the project's source assets\n\
           sync-script-api  refresh the engine-owned versioned C# gameplay contract\n\
           build-scripts  compile C# scripts and publish the script host\n\
           build    cook assets and compile configured scripts\n\
           run      run the startup scene\n\
           editor   open the project in the editor"
    );
}
