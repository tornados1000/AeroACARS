<?php

//use Illuminate\Support\Facades\DB;
//use Illuminate\Support\Facades\Schema;
use Modules\VMSAcars\Contracts\Migration;

return new class() extends Migration {
    public function up()
    {
        $perms = [
            [
                'name'=>'acars_admin',
                'display_name'=>'ACARS Admin',
                'description'=>'ACARS Admin',
            ],
            [
                'name' => 'acars_operations',
                'display_name' => 'ACARS Operations',
                'description' => 'ACARS Operations',
            ]
        ];

        foreach ($perms as $perm) {
            try {
                \App\Models\Permission::create($perm);
            } catch (Exception $e) {
                continue;
            }
        }

        /*if (class_exists('Spatie\Permission\Models\Permission')) {
            app()[\Spatie\Permission\PermissionRegistrar::class]->forgetCachedPermissions();
            try {
                foreach ($perms as $perm) {
                    \Spatie\Permission\Models\Permission::create(['name' => $perm['name']]);
                }
            } catch (\Spatie\Permission\Exceptions\PermissionAlreadyExists $e) {
                Log::info('Permission already exists: your_permission_name');
            }

            app()[\Spatie\Permission\PermissionRegistrar::class]->forgetCachedPermissions();
        } else {
            foreach ($perms as $perm) {
                \App\Models\Permission::create($perm);
            }
        }*/
    }

    public function down()
    {
    }
};
