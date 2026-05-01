<?php

namespace Modules\VMSAcars\Services;

use App\Models\Kvp;
use App\Models\User;
use App\Models\UserField;
use App\Models\UserFieldValue;
use App\Support\Utils;
use GuzzleHttp\Client as GuzzleClient;
use Illuminate\Support\Facades\Log;
use Illuminate\Support\Facades\Schema;

class AcarsService
{
    /**
     * @return bool|array False if failed
     */
    public function validateLicense(string $licenceKey)
    {
        $domain = Utils::getRootDomain(config('app.url'));

        Log::info('Validating ACARS license, domain='.$domain.', key='.$licenceKey);

        $client = new GuzzleClient(['base_uri' => config('vmsacars.api_url')]);
        $response = $client->post('/v1/acars/license', [
            'json' => [
                'key'    => $licenceKey,
                'domain' => $domain,
            ],
        ]);

        $body = $response->getBody()->getContents();

        if ($response->getStatusCode() !== 200) {
            Log::error('Error with license: '.$body);

            return ['error' => true, 'message' => $body];
        }

        $body = json_decode($body, false);

        $results = [
            'error'   => false,
            'active'  => $body->a,
            'premium' => $body->p,
        ];

        if ($results['active'] !== true) {
            Log::error('Error in license validation='.$body);
            $results['error'] = true;
        }

        return $results;
    }

    public function sendBroadcast(User $user, string $message): array
    {
        $data = [
            'AirlineId' => $this->getAirlineId(),
            'Message'   => $message,
            'UserId'    => $this->getUserId($user),
        ];

        $client = new GuzzleClient(['base_uri' => config('vmsacars.api_url')]);
        $response = $client->post('/v1/acars/broadcast', [
            'json'    => $data,
            'headers' => [
                'X-Airline-Id' => $data['AirlineId'],
                'X-User-Id'    => $data['UserId'],
            ],
        ]);

        $body = $response->getBody()->getContents();

        if ($response->getStatusCode() !== 200) {
            Log::error('Error with sending license: '.$body);

            return ['error' => true, 'message' => $body];
        }

        return ['error' => false, 'message' => 'Message sent!'];
    }

    /**
     * Return an ID for this airline, which won't change
     */
    public function getAirlineId(): string
    {
        $where = [
            'key' => 'acars_airline_id',
        ];

        $value = Kvp::where($where)->firstOr(function () use ($where) {
            $fields = array_merge($where, [
                'value' => Utils::generateNewId(18),
            ]);

            return Kvp::create($fields);
        });

        return $value->value;
    }

    /**
     * Return a user ID that's unique and stays the same
     */
    public function getUserId(User $user): string
    {
        $insert = [
            'description'          => 'User ID for acars/vacentral. DO NOT DELETE',
            'show_on_registration' => false,
            'required'             => false,
            'private'              => true,
        ];

        if (Schema::hasColumn('user_fields', 'internal')) {
            $insert['internal'] = true;
        }

        // Add the user ID...
        $field_record = UserField::firstOrCreate(
            ['name' => 'acars_user_id'],
            $insert
        );

        $field_id = $field_record->id;
        $where = [
            'user_field_id' => $field_id,
            'user_id'       => $user->id,
        ];

        $value = UserFieldValue::where($where)->firstOr(function () use ($where) {
            $fields = array_merge($where, [
                'value' => Utils::generateNewId(18),
            ]);

            return UserFieldValue::create($fields);
        });

        return $value->value;
    }
}
